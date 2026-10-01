use std::{cell::RefCell, ops::Deref, rc::Rc};

use libpulse_binding as pulse;
use pulse::{callbacks::ListResult, context::introspect::SinkInfo, mainloop::threaded::Mainloop};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VolumeState {
    pub percent: u8,
    pub muted: bool,
}

pub trait VolumeProvider {
    fn read(&mut self) -> Result<VolumeState, String>;
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
        self.mainloop.borrow_mut().unlock();
        sink.borrow_mut()
            .take()
            .ok_or_else(|| "could not read the default PulseAudio sink".to_string())
    }
}

impl VolumeProvider for PulseAudio {
    fn read(&mut self) -> Result<VolumeState, String> {
        let sink = self.read_default_sink()?;
        let percent = volume_percent(sink.volume.avg().0);
        Ok(VolumeState {
            percent,
            muted: sink.mute,
        })
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
