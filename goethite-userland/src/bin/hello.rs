#![no_std]
#![no_main]

use goethite_userland::println;

#[unsafe(no_mangle)]
pub fn main() {
    println!("Hello, world!");
}