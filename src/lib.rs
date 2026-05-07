#![deny(dead_code)]
#![deny(unused_imports)]
#![deny(unused_variables)]
#![deny(unused_mut)]
#![deny(unused_must_use)]
#![deny(unreachable_code)]
#![deny(unreachable_patterns)]
#![deny(unsafe_op_in_unsafe_fn)]
#![deny(missing_debug_implementations)]
#![forbid(trivial_casts)]
#![cfg_attr(test, allow(dead_code))]

pub mod bridge;
pub mod carve;
pub mod custody;
pub mod error;
pub mod io;
pub mod reassemble;
pub mod validate;
