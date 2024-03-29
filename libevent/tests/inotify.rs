#[cfg(test)]
mod test {
    use nix::sys::inotify::AddWatchFlags;
    use std::os::unix::prelude::RawFd;
    use std::path::Path;
    use std::rc::Rc;
    use utils::Error;

    use event::Events;
    use event::Source;
    use event::{EventState, EventType};

    #[derive(Debug)]
    struct Timer();

    impl Timer {
        fn new() -> Timer {
            Self {}
        }
    }

    impl Source for Timer {
        fn fd(&self) -> RawFd {
            0
        }

        fn event_type(&self) -> EventType {
            EventType::Inotify
        }

        fn epoll_event(&self) -> u32 {
            (libc::EPOLLIN) as u32
        }

        fn priority(&self) -> i8 {
            0i8
        }

        fn dispatch(&self, e: &Events) -> Result<i32, Error> {
            println!("Dispatching inotify!");
            e.set_exit();
            Ok(0)
        }

        fn token(&self) -> u64 {
            let data: u64 = unsafe { std::mem::transmute(self) };
            data
        }
    }

    #[test]
    #[ignore]
    fn test_timer() {
        let e = Events::new().unwrap();
        let s: Rc<dyn Source> = Rc::new(Timer::new());
        e.add_source(s.clone()).unwrap();

        e.set_enabled(s.clone(), EventState::On).unwrap();

        let watch = Path::new("/");
        let wd = e.add_watch(watch, AddWatchFlags::IN_ALL_EVENTS);

        e.rloop().unwrap();

        e.rm_watch(wd);

        e.del_source(s.clone()).unwrap();
    }
}
