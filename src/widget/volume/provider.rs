use std::{cell::RefCell, ops::Deref, rc::Rc};

use libpulse_binding as pulse;
use pulse::{
    callbacks::ListResult,
    context::introspect::{SinkInfo, SourceInfo},
    mainloop::threaded::Mainloop,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeState {
    pub percent: u8,
    pub muted: bool,
    pub microphone: Option<MicrophoneState>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MicrophoneState {
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

    fn set_microphone_muted(&mut self, _muted: bool) -> Result<(), String> {
        Err("volume provider does not support muting the microphone".to_string())
    }

    fn set_microphone_percent(&mut self, _percent: u8) -> Result<(), String> {
        Err("volume provider does not support changing microphone volume".to_string())
    }
}

pub fn create(name: &str) -> Result<Box<dyn VolumeProvider>, String> {
    match name {
        "pulseaudio" | "pulse" => Ok(Box::new(PulseAudio::new()?)),
        _ => Err(format!("unknown volume provider `{name}`")),
    }
}

pub struct Unavailable {
    pub error: String,
}

impl VolumeProvider for Unavailable {
    fn read(&mut self) -> Result<VolumeState, String> {
        Err(self.error.clone())
    }
}

pub struct PulseAudio {
    context: Rc<RefCell<pulse::context::Context>>,
    mainloop: Rc<RefCell<Mainloop>>,
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

        Ok(Self { mainloop, context })
    }

    fn read_default_sink(&mut self) -> Result<SinkInfo<'static>, String> {
        self.mainloop.borrow_mut().lock();
        let default_name = Rc::new(RefCell::new(None));
        let default_name_ref = Rc::clone(&default_name);
        let mainloop_ref = Rc::clone(&self.mainloop);
        let operation = self
            .context
            .borrow()
            .introspect()
            .get_server_info(move |info| {
                *default_name_ref.borrow_mut() =
                    info.default_sink_name.as_ref().map(ToString::to_string);
                unsafe { (*mainloop_ref.as_ptr()).signal(false) };
            });
        while operation.get_state() != pulse::operation::State::Done {
            self.mainloop.borrow_mut().wait();
        }
        let Some(name) = default_name.borrow_mut().take() else {
            self.mainloop.borrow_mut().unlock();
            return Err("PulseAudio has no default sink".to_string());
        };

        let sink = Rc::new(RefCell::new(None));
        let sink_ref = Rc::clone(&sink);
        let mainloop_ref = Rc::clone(&self.mainloop);
        let operation =
            self.context
                .borrow()
                .introspect()
                .get_sink_info_by_name(&name, move |result| {
                    if let ListResult::Item(info) = result {
                        *sink_ref.borrow_mut() = Some(info.to_owned());
                    }
                    unsafe { (*mainloop_ref.as_ptr()).signal(false) };
                });
        while operation.get_state() != pulse::operation::State::Done {
            self.mainloop.borrow_mut().wait();
        }
        if operation.get_state() == pulse::operation::State::Cancelled {
            self.mainloop.borrow_mut().unlock();
            return Err("PulseAudio sink lookup operation was cancelled".to_string());
        }
        self.mainloop.borrow_mut().unlock();
        sink.borrow_mut()
            .take()
            .ok_or_else(|| "could not read the default PulseAudio sink".to_string())
    }

    fn read_default_source(&mut self) -> Result<SourceInfo<'static>, String> {
        self.mainloop.borrow_mut().lock();
        let default_name = Rc::new(RefCell::new(None));
        let default_name_ref = Rc::clone(&default_name);
        let mainloop_ref = Rc::clone(&self.mainloop);
        let operation = self
            .context
            .borrow()
            .introspect()
            .get_server_info(move |info| {
                *default_name_ref.borrow_mut() =
                    info.default_source_name.as_ref().map(ToString::to_string);
                unsafe { (*mainloop_ref.as_ptr()).signal(false) };
            });
        while operation.get_state() != pulse::operation::State::Done {
            self.mainloop.borrow_mut().wait();
        }
        let Some(name) = default_name.borrow_mut().take() else {
            self.mainloop.borrow_mut().unlock();
            return Err("PulseAudio has no default source".to_string());
        };

        let source = Rc::new(RefCell::new(None));
        let source_ref = Rc::clone(&source);
        let mainloop_ref = Rc::clone(&self.mainloop);
        let operation =
            self.context
                .borrow()
                .introspect()
                .get_source_info_by_name(&name, move |result| {
                    if let ListResult::Item(info) = result {
                        *source_ref.borrow_mut() = Some(info.to_owned());
                    }
                    unsafe { (*mainloop_ref.as_ptr()).signal(false) };
                });
        while operation.get_state() != pulse::operation::State::Done {
            self.mainloop.borrow_mut().wait();
        }
        self.mainloop.borrow_mut().unlock();
        source
            .borrow_mut()
            .take()
            .ok_or_else(|| "could not read the default PulseAudio source".to_string())
    }
}

impl VolumeProvider for PulseAudio {
    fn read(&mut self) -> Result<VolumeState, String> {
        let sink = self.read_default_sink()?;
        let percent = volume_percent(sink.volume.avg().0);
        let microphone = self
            .read_default_source()
            .ok()
            .map(|source| MicrophoneState {
                percent: volume_percent(source.volume.avg().0),
                muted: source.mute,
            });
        Ok(VolumeState {
            percent,
            muted: sink.mute,
            microphone,
        })
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
        while operation.get_state() != pulse::operation::State::Done {
            self.mainloop.borrow_mut().wait();
        }
        if operation.get_state() == pulse::operation::State::Cancelled {
            self.mainloop.borrow_mut().unlock();
            return Err("PulseAudio volume operation was cancelled".to_string());
        }
        self.mainloop.borrow_mut().unlock();
        if result.borrow_mut().take().unwrap_or(false) {
            Ok(())
        } else {
            Err(format!("PulseAudio rejected setting mute to {muted}"))
        }
    }

    fn set_percent(&mut self, percent: u8) -> Result<(), String> {
        let sink = self.read_default_sink()?;
        let value =
            ((percent as f64 / 100.0) * pulse::volume::Volume::NORMAL.0 as f64).round() as u32;
        let mut volumes = sink.volume;
        volumes.set(volumes.len(), pulse::volume::Volume(value));

        self.mainloop.borrow_mut().lock();
        let mainloop_ref = Rc::clone(&self.mainloop);
        let operation = self.context.borrow().introspect().set_sink_volume_by_index(
            sink.index,
            &volumes,
            Some(Box::new(move |_| {
                unsafe { (*mainloop_ref.as_ptr()).signal(false) };
            })),
        );
        while operation.get_state() != pulse::operation::State::Done {
            self.mainloop.borrow_mut().wait();
        }
        if operation.get_state() == pulse::operation::State::Cancelled {
            self.mainloop.borrow_mut().unlock();
            return Err("PulseAudio volume operation was cancelled".to_string());
        }
        self.mainloop.borrow_mut().unlock();
        Ok(())
    }

    fn set_microphone_muted(&mut self, muted: bool) -> Result<(), String> {
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
        while operation.get_state() != pulse::operation::State::Done {
            self.mainloop.borrow_mut().wait();
        }
        let cancelled = operation.get_state() == pulse::operation::State::Cancelled;
        self.mainloop.borrow_mut().unlock();
        if cancelled {
            Err("PulseAudio microphone mute operation was cancelled".to_string())
        } else {
            Ok(())
        }
    }

    fn set_microphone_percent(&mut self, percent: u8) -> Result<(), String> {
        let source = self.read_default_source()?;
        let value =
            ((percent as f64 / 100.0) * pulse::volume::Volume::NORMAL.0 as f64).round() as u32;
        let mut volumes = source.volume;
        volumes.set(volumes.len(), pulse::volume::Volume(value));

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
        while operation.get_state() != pulse::operation::State::Done {
            self.mainloop.borrow_mut().wait();
        }
        let cancelled = operation.get_state() == pulse::operation::State::Cancelled;
        self.mainloop.borrow_mut().unlock();
        if cancelled {
            Err("PulseAudio microphone volume operation was cancelled".to_string())
        } else {
            Ok(())
        }
    }
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
