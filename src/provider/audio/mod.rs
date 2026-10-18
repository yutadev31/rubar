use std::{
    cell::RefCell,
    ops::Deref,
    rc::Rc,
    sync::{Arc, Mutex},
};

use libpulse_binding as pulse;
use pulse::{
    callbacks::ListResult,
    context::introspect::{SinkInfo, SourceInfo},
    context::subscribe::{self, InterestMaskSet},
    mainloop::threaded::Mainloop,
    operation::{Operation, State as OperationState},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeState {
    pub output: Option<VolumeChannelState>,
    pub input: Option<VolumeChannelState>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeChannelState {
    pub percent: u8,
    pub muted: bool,
}

pub trait VolumeProvider {
    fn read(&mut self) -> Result<VolumeState, String>;

    fn set_muted(&mut self, _muted: bool) -> Result<(), String> {
        Err("volume provider does not support muting".to_string())
    }

    fn set_percent(&mut self, _percent: u8) -> Result<(), String> {
        Err("volume provider does not support changing volume".to_string())
    }

    fn set_in_muted(&mut self, _muted: bool) -> Result<(), String> {
        Err("volume provider does not support muting the input".to_string())
    }

    fn set_in_percent(&mut self, _percent: u8) -> Result<(), String> {
        Err("volume provider does not support changing input volume".to_string())
    }
}

pub fn create(name: &str) -> Result<Box<dyn VolumeProvider>, String> {
    match name {
        "pulseaudio" | "pulse" => Ok(Box::new(PulseAudio::new()?)),
        _ => Err(format!("unknown volume provider `{name}`")),
    }
}

pub struct PulseAudio {
    context: Rc<RefCell<pulse::context::Context>>,
    mainloop: Rc<RefCell<Mainloop>>,
    state: Arc<Mutex<Option<VolumeState>>>,
}

#[derive(Clone, Copy)]
enum DeviceKind {
    Sink,
    Source,
}

impl DeviceKind {
    fn name(self) -> &'static str {
        match self {
            Self::Sink => "sink",
            Self::Source => "source",
        }
    }
}

enum DeviceInfo {
    Sink(SinkInfo<'static>),
    Source(SourceInfo<'static>),
}

fn complete_operation<C: ?Sized>(
    mainloop: &Rc<RefCell<Mainloop>>,
    operation: &Operation<C>,
    cancelled_message: &str,
) -> Result<(), String> {
    while operation.get_state() == OperationState::Running {
        mainloop.borrow_mut().wait();
    }
    let cancelled = operation.get_state() == OperationState::Cancelled;
    mainloop.borrow_mut().unlock();
    if cancelled {
        Err(cancelled_message.to_string())
    } else {
        Ok(())
    }
}

fn pulse_volume(percent: u8) -> pulse::volume::Volume {
    let value =
        ((f64::from(percent) / 100.0) * f64::from(pulse::volume::Volume::NORMAL.0)).round() as u32;
    pulse::volume::Volume(value)
}

impl PulseAudio {
    fn new() -> Result<Self, String> {
        let mainloop =
            Rc::new(RefCell::new(Mainloop::new().ok_or_else(|| {
                "could not create PulseAudio mainloop".to_string()
            })?));
        let context = Rc::new(RefCell::new(
            pulse::context::Context::new(mainloop.borrow().deref(), "rubar")
                .ok_or_else(|| "could not create PulseAudio context".to_string())?,
        ));

        let mainloop_ref = Rc::clone(&mainloop);
        let context_ref = Rc::clone(&context);
        context
            .borrow_mut()
            .set_state_callback(Some(Box::new(move || {
                let state = unsafe { (*context_ref.as_ptr()).get_state() };
                if matches!(
                    state,
                    pulse::context::State::Ready
                        | pulse::context::State::Failed
                        | pulse::context::State::Terminated
                ) {
                    unsafe { (*mainloop_ref.as_ptr()).signal(false) };
                }
            })));
        context
            .borrow_mut()
            .connect(None, pulse::context::FlagSet::NOFLAGS, None)
            .map_err(|error| format!("could not connect to PulseAudio: {error:?}"))?;

        mainloop.borrow_mut().lock();
        mainloop
            .borrow_mut()
            .start()
            .map_err(|error| format!("could not start PulseAudio mainloop: {error:?}"))?;
        loop {
            match context.borrow().get_state() {
                pulse::context::State::Ready => break,
                pulse::context::State::Failed | pulse::context::State::Terminated => {
                    mainloop.borrow_mut().unlock();
                    mainloop.borrow_mut().stop();
                    return Err("PulseAudio context failed to connect".to_string());
                }
                _ => mainloop.borrow_mut().wait(),
            }
        }
        context.borrow_mut().set_state_callback(None);
        mainloop.borrow_mut().unlock();

        let state = Arc::new(Mutex::new(None));
        install_subscription(&context, &mainloop, &state)?;

        Ok(Self {
            mainloop,
            context,
            state,
        })
    }

    fn read_default_sink(&mut self) -> Result<SinkInfo<'static>, String> {
        match self.read_default_device(DeviceKind::Sink)? {
            DeviceInfo::Sink(info) => Ok(info),
            DeviceInfo::Source(_) => unreachable!("sink lookup returned a source"),
        }
    }

    fn read_default_source(&mut self) -> Result<SourceInfo<'static>, String> {
        match self.read_default_device(DeviceKind::Source)? {
            DeviceInfo::Source(info) => Ok(info),
            DeviceInfo::Sink(_) => unreachable!("source lookup returned a sink"),
        }
    }

    fn read_default_device(&mut self, kind: DeviceKind) -> Result<DeviceInfo, String> {
        self.mainloop.borrow_mut().lock();
        let default_name = Rc::new(RefCell::new(None));
        let default_name_ref = Rc::clone(&default_name);
        let mainloop_ref = Rc::clone(&self.mainloop);
        let operation = self
            .context
            .borrow()
            .introspect()
            .get_server_info(move |info| {
                *default_name_ref.borrow_mut() = match kind {
                    DeviceKind::Sink => info.default_sink_name.as_ref(),
                    DeviceKind::Source => info.default_source_name.as_ref(),
                }
                .map(ToString::to_string);
                unsafe { (*mainloop_ref.as_ptr()).signal(false) };
            });
        complete_operation(
            &self.mainloop,
            &operation,
            "PulseAudio server info operation was cancelled",
        )?;
        let Some(name) = default_name.borrow_mut().take() else {
            return Err(format!("PulseAudio has no default {}", kind.name()));
        };

        let device = Rc::new(RefCell::new(None));
        let mainloop_ref = Rc::clone(&self.mainloop);
        self.mainloop.borrow_mut().lock();
        match kind {
            DeviceKind::Sink => {
                let device_ref = Rc::clone(&device);
                let operation = self.context.borrow().introspect().get_sink_info_by_name(
                    &name,
                    move |result| {
                        if let ListResult::Item(info) = result {
                            *device_ref.borrow_mut() = Some(DeviceInfo::Sink(info.to_owned()));
                        }
                        unsafe { (*mainloop_ref.as_ptr()).signal(false) };
                    },
                );
                complete_operation(
                    &self.mainloop,
                    &operation,
                    "PulseAudio sink lookup operation was cancelled",
                )?;
            }
            DeviceKind::Source => {
                let device_ref = Rc::clone(&device);
                let operation = self.context.borrow().introspect().get_source_info_by_name(
                    &name,
                    move |result| {
                        if let ListResult::Item(info) = result {
                            *device_ref.borrow_mut() = Some(DeviceInfo::Source(info.to_owned()));
                        }
                        unsafe { (*mainloop_ref.as_ptr()).signal(false) };
                    },
                );
                complete_operation(
                    &self.mainloop,
                    &operation,
                    "PulseAudio source lookup operation was cancelled",
                )?;
            }
        }
        device
            .borrow_mut()
            .take()
            .ok_or_else(|| format!("could not read the default PulseAudio {}", kind.name()))
    }
}

impl VolumeProvider for PulseAudio {
    fn read(&mut self) -> Result<VolumeState, String> {
        if let Some(state) = *self.state.lock().unwrap() {
            return Ok(state);
        }

        let sink = self.read_default_sink()?;
        let percent = volume_percent(sink.volume.avg().0);
        let input = match self.read_default_source() {
            Ok(source) => Some(VolumeChannelState {
                percent: volume_percent(source.volume.avg().0),
                muted: source.mute,
            }),
            Err(error) => {
                tracing::warn!("rubar: could not read default PulseAudio source: {error}");
                None
            }
        };
        let state = VolumeState {
            output: Some(VolumeChannelState {
                percent,
                muted: sink.mute,
            }),
            input,
        };
        *self.state.lock().unwrap() = Some(state);
        Ok(state)
    }

    fn set_muted(&mut self, muted: bool) -> Result<(), String> {
        let sink = self.read_default_sink()?;
        self.mainloop.borrow_mut().lock();
        let result = Rc::new(RefCell::new(None));
        let result_ref = Rc::clone(&result);
        let mainloop_ref = Rc::clone(&self.mainloop);
        let operation = self.context.borrow().introspect().set_sink_mute_by_index(
            sink.index,
            muted,
            Some(Box::new(move |success| {
                *result_ref.borrow_mut() = Some(success);
                unsafe { (*mainloop_ref.as_ptr()).signal(false) };
            })),
        );
        complete_operation(
            &self.mainloop,
            &operation,
            "PulseAudio volume operation was cancelled",
        )?;
        if result.borrow_mut().take().unwrap_or(false) {
            Ok(())
        } else {
            Err(format!("PulseAudio rejected setting mute to {muted}"))
        }
    }

    fn set_percent(&mut self, percent: u8) -> Result<(), String> {
        let sink = self.read_default_sink()?;
        let mut volumes = sink.volume;
        volumes.set(volumes.len(), pulse_volume(percent));

        self.mainloop.borrow_mut().lock();
        let mainloop_ref = Rc::clone(&self.mainloop);
        let operation = self.context.borrow().introspect().set_sink_volume_by_index(
            sink.index,
            &volumes,
            Some(Box::new(move |_| {
                unsafe { (*mainloop_ref.as_ptr()).signal(false) };
            })),
        );
        complete_operation(
            &self.mainloop,
            &operation,
            "PulseAudio volume operation was cancelled",
        )
    }

    fn set_in_muted(&mut self, muted: bool) -> Result<(), String> {
        let source = self.read_default_source()?;
        self.mainloop.borrow_mut().lock();
        let mainloop_ref = Rc::clone(&self.mainloop);
        let operation = self.context.borrow().introspect().set_source_mute_by_index(
            source.index,
            muted,
            Some(Box::new(move |_| {
                unsafe { (*mainloop_ref.as_ptr()).signal(false) };
            })),
        );
        complete_operation(
            &self.mainloop,
            &operation,
            "PulseAudio input mute operation was cancelled",
        )
    }

    fn set_in_percent(&mut self, percent: u8) -> Result<(), String> {
        let source = self.read_default_source()?;
        let mut volumes = source.volume;
        volumes.set(volumes.len(), pulse_volume(percent));

        self.mainloop.borrow_mut().lock();
        let mainloop_ref = Rc::clone(&self.mainloop);
        let operation = self
            .context
            .borrow()
            .introspect()
            .set_source_volume_by_index(
                source.index,
                &volumes,
                Some(Box::new(move |_| {
                    unsafe { (*mainloop_ref.as_ptr()).signal(false) };
                })),
            );
        complete_operation(
            &self.mainloop,
            &operation,
            "PulseAudio input volume operation was cancelled",
        )
    }
}

fn install_subscription(
    context: &Rc<RefCell<pulse::context::Context>>,
    mainloop: &Rc<RefCell<Mainloop>>,
    state: &Arc<Mutex<Option<VolumeState>>>,
) -> Result<(), String> {
    let context_weak = Rc::downgrade(context);
    let state = Arc::clone(state);

    mainloop.borrow_mut().lock();
    context.borrow_mut().set_subscribe_callback(Some(Box::new(
        move |facility, operation, _index| {
            if !matches!(
                facility,
                Some(
                    subscribe::Facility::Sink
                        | subscribe::Facility::Source
                        | subscribe::Facility::Server
                )
            ) || !matches!(
                operation,
                Some(
                    subscribe::Operation::New
                        | subscribe::Operation::Changed
                        | subscribe::Operation::Removed
                )
            ) {
                return;
            }

            let Some(context) = context_weak.upgrade() else {
                return;
            };
            refresh_async(&context, &state);
        },
    )));

    let completed = Rc::new(RefCell::new(None));
    let completed_ref = Rc::clone(&completed);
    let mainloop_ref = Rc::downgrade(mainloop);
    let operation = context.borrow_mut().subscribe(
        InterestMaskSet::SINK | InterestMaskSet::SOURCE | InterestMaskSet::SERVER,
        move |success| {
            *completed_ref.borrow_mut() = Some(success);
            if let Some(mainloop) = mainloop_ref.upgrade() {
                unsafe { (*mainloop.as_ptr()).signal(false) };
            }
        },
    );
    while operation.get_state() != pulse::operation::State::Done {
        mainloop.borrow_mut().wait();
    }
    let success = completed.borrow_mut().take().unwrap_or(false);
    let cancelled = operation.get_state() == pulse::operation::State::Cancelled;
    mainloop.borrow_mut().unlock();

    if cancelled || !success {
        Err("could not subscribe to PulseAudio volume events".to_string())
    } else {
        Ok(())
    }
}

fn refresh_async(
    context: &Rc<RefCell<pulse::context::Context>>,
    state: &Arc<Mutex<Option<VolumeState>>>,
) {
    let state_for_server = Arc::clone(state);
    let context_for_server = Rc::clone(context);
    context.borrow().introspect().get_server_info(move |info| {
        let sink_name = info.default_sink_name.as_ref().map(ToString::to_string);
        let source_name = info.default_source_name.as_ref().map(ToString::to_string);

        if let Some(name) = sink_name {
            let state = Arc::clone(&state_for_server);
            context_for_server
                .borrow()
                .introspect()
                .get_sink_info_by_name(&name, move |result| {
                    if let ListResult::Item(info) = result {
                        let mut current = state.lock().unwrap();
                        let input = current.and_then(|state| state.input);
                        *current = Some(VolumeState {
                            output: Some(VolumeChannelState {
                                percent: volume_percent(info.volume.avg().0),
                                muted: info.mute,
                            }),
                            input,
                        });
                    }
                });
        }

        if let Some(name) = source_name {
            let state = Arc::clone(&state_for_server);
            context_for_server
                .borrow()
                .introspect()
                .get_source_info_by_name(&name, move |result| {
                    if let ListResult::Item(info) = result {
                        let mut current = state.lock().unwrap();
                        let output = current.and_then(|state| state.output);
                        *current = Some(VolumeState {
                            output,
                            input: Some(VolumeChannelState {
                                percent: volume_percent(info.volume.avg().0),
                                muted: info.mute,
                            }),
                        });
                    }
                });
        }
    });
}

fn volume_percent(value: u32) -> u8 {
    ((value as f64 / pulse::volume::Volume::NORMAL.0 as f64) * 100.0)
        .round()
        .clamp(0.0, 255.0) as u8
}

#[cfg(test)]
mod tests {
    use super::volume_percent;

    #[test]
    fn converts_pulse_volume_using_normal_as_one_hundred_percent() {
        assert_eq!(volume_percent(0), 0);
        assert_eq!(volume_percent(32_768), 50);
        assert_eq!(volume_percent(65_536), 100);
    }
}
