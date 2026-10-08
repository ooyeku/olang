//! The crate's `print!`, `println!`, `eprint!` and `eprintln!`: `std`'s,
//! except that they never panic (see `stdio.rs`). Declared first with
//! `#[macro_use]` in `lib.rs` and in `main.rs` (whose `stdio` module is
//! the library's), they shadow `std`'s in every module of the crate.

#[allow(unused_macros)]
macro_rules! print {
    ($($arg:tt)*) => {
        $crate::stdio::out(format_args!($($arg)*))
    };
}

#[allow(unused_macros)]
macro_rules! println {
    () => {
        $crate::stdio::out(format_args!("\n"))
    };
    ($($arg:tt)*) => {
        $crate::stdio::out_line(format_args!($($arg)*))
    };
}

#[allow(unused_macros)]
macro_rules! eprint {
    ($($arg:tt)*) => {
        $crate::stdio::err(format_args!($($arg)*))
    };
}

#[allow(unused_macros)]
macro_rules! eprintln {
    () => {
        $crate::stdio::err(format_args!("\n"))
    };
    ($($arg:tt)*) => {
        $crate::stdio::err_line(format_args!($($arg)*))
    };
}
