#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use nih_plug::prelude::*;
use phantom_spire::PhantomSpire;

fn main() {
    nih_export_standalone::<PhantomSpire>();
}
