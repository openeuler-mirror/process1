#![allow(unused_macros)]
#[macro_export]
macro_rules! syscall {
    ($fn: ident ( $($arg: expr),* $(,)* ) ) => {{
        let res = unsafe { libc::$fn($($arg, )*) };
        if res < 0 {
            utils::Result::Err(utils::Error::Syscall { syscall: stringify!($fn), errno: unsafe { *libc::__errno_location() }, ret: res })
        } else {
            utils::Result::Ok(res)
        }
    }};
}

#[macro_export]
macro_rules! IN_SET {
    ($ov:expr, $($nv:expr),+) => {
        {
            let mut found = false;
            $(
                if $ov == $nv {
                    found = true;
                }
            )+

            found
        }
    };
}
