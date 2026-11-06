use image::RgbaImage;

pub fn reverse_print(mask: &RgbaImage, background: &mut RgbaImage) {
    let (width, height) = mask.dimensions();
    let alpha_threshold = 30u8;

    for y in 0..height {
        for x in 0..width {
            let m = mask.get_pixel(x, y);
            if m[3] < alpha_threshold {
                continue;
            }
            let bg = background.get_pixel(x, y);
            background.put_pixel(
                x,
                y,
                image::Rgba([255 - bg[0], 255 - bg[1], 255 - bg[2], bg[3]]),
            );
        }
    }
}
