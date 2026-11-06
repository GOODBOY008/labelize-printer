use image::{Rgba, RgbaImage};

#[derive(Clone, Debug)]
pub struct BitMatrix {
    data: Vec<bool>,
    width: usize,
    height: usize,
}

impl BitMatrix {
    pub fn new(width: usize, height: usize) -> Self {
        BitMatrix {
            data: vec![false; width * height],
            width,
            height,
        }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn get(&self, x: usize, y: usize) -> bool {
        if x < self.width && y < self.height {
            self.data[y * self.width + x]
        } else {
            false
        }
    }

    pub fn set(&mut self, x: usize, y: usize, val: bool) {
        if x < self.width && y < self.height {
            self.data[y * self.width + x] = val;
        }
    }

    /// Set a horizontal range of bits starting at (x, y=0) for `count` pixels.
    pub fn set_range(&mut self, x: usize, count: usize, val: bool) {
        for i in 0..count {
            self.set(x + i, 0, val);
        }
    }

    pub fn to_image(&self, scale_x: usize, scale_y: usize) -> RgbaImage {
        let iw = self.width * scale_x;
        let ih = self.height * scale_y;
        let mut img = RgbaImage::from_pixel(iw as u32, ih as u32, Rgba([0, 0, 0, 0]));

        for y in 0..self.height {
            for x in 0..self.width {
                if self.get(x, y) {
                    for sy in 0..scale_y {
                        for sx in 0..scale_x {
                            let px = (x * scale_x + sx) as u32;
                            let py = (y * scale_y + sy) as u32;
                            if px < img.width() && py < img.height() {
                                img.put_pixel(px, py, Rgba([0, 0, 0, 255]));
                            }
                        }
                    }
                }
            }
        }

        img
    }

    /// Create a 1D barcode image from a bool slice (bar pattern), scaled to given height and bar width.
    pub fn to_1d_image(&self, bar_width: usize, height: usize) -> RgbaImage {
        let iw = self.width * bar_width;
        let mut img = RgbaImage::from_pixel(iw as u32, height as u32, Rgba([0, 0, 0, 0]));

        for x in 0..self.width {
            if self.get(x, 0) {
                for bw in 0..bar_width {
                    let px = (x * bar_width + bw) as u32;
                    for py in 0..height as u32 {
                        if px < img.width() {
                            img.put_pixel(px, py, Rgba([0, 0, 0, 255]));
                        }
                    }
                }
            }
        }

        img
    }
}
