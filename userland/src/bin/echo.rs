#![no_std]
#![no_main]

use goethite_userland::{args, print, println};

#[unsafe(no_mangle)]
pub fn main() {
    if let Some(iter) = args() {
        let mut first = true;
        for arg in iter.skip(1) {
            if !first {
                print!(" ");
            }
            print!("{}", arg);
            first = false;
        }
    }
    println!();
}
