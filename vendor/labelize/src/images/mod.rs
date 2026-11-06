pub mod monochrome;
pub mod pdf;
pub mod reverse_print;
pub mod scaled;

use image::Rgba;

pub const COLOR_BLACK: Rgba<u8> = Rgba([0, 0, 0, 255]);
pub const COLOR_WHITE: Rgba<u8> = Rgba([255, 255, 255, 255]);
pub const COLOR_TRANSPARENT: Rgba<u8> = Rgba([0, 0, 0, 0]);
