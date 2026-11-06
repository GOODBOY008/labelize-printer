use super::field_orientation::FieldOrientation;

#[derive(Clone, Debug)]
pub struct GraphicSymbol {
    pub width: f64,
    pub height: f64,
    pub orientation: FieldOrientation,
}
