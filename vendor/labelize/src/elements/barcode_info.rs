#[derive(Clone, Debug)]
pub struct BarcodeDimensions {
    pub module_width: i32,
    pub height: i32,
    pub width_ratio: f64,
}

impl Default for BarcodeDimensions {
    fn default() -> Self {
        BarcodeDimensions {
            module_width: 2,
            height: 10,
            width_ratio: 3.0,
        }
    }
}
