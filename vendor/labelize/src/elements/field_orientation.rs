#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FieldOrientation {
    #[default]
    Normal = 0,
    Rotated90 = 1,
    Rotated180 = 2,
    Rotated270 = 3,
}

impl FieldOrientation {
    pub fn get_degrees(&self) -> f64 {
        match self {
            FieldOrientation::Normal => 0.0,
            FieldOrientation::Rotated90 => 90.0,
            FieldOrientation::Rotated180 => 180.0,
            FieldOrientation::Rotated270 => 270.0,
        }
    }
}
