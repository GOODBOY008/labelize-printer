use super::field_orientation::FieldOrientation;
use super::label_position::LabelPosition;
use super::reverse_print::ReversePrint;

#[derive(Clone, Debug)]
pub struct BarcodeAztec {
    pub orientation: FieldOrientation,
    pub magnification: i32,
    pub size: i32,
}

#[derive(Clone, Debug)]
pub struct BarcodeAztecWithData {
    pub reverse_print: ReversePrint,
    pub barcode: BarcodeAztec,
    pub position: LabelPosition,
    pub data: String,
}
