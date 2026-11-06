use image::RgbaImage;

pub fn new_scaled(img: &RgbaImage, scale_x: u32, scale_y: u32) -> RgbaImage {
    if scale_x == 1 && scale_y == 1 {
        return img.clone();
    }

    let (w, h) = img.dimensions();
    let nw = w * scale_x;
    let nh = h * scale_y;
    let mut dst = RgbaImage::new(nw, nh);

    for y in 0..nh {
        for x in 0..nw {
            let px = img.get_pixel(x / scale_x, y / scale_y);
            dst.put_pixel(x, y, *px);
        }
    }

    dst
}

pub fn new_scaled_1d_height(img: &RgbaImage, scale_y: u32) -> RgbaImage {
    let (w, _h) = img.dimensions();
    let mut dst = RgbaImage::new(w, scale_y);

    for y in 0..scale_y {
        for x in 0..w {
            let px = img.get_pixel(x, 0);
            dst.put_pixel(x, y, *px);
        }
    }

    dst
}
