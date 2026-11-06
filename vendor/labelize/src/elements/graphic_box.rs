use super::label_position::LabelPosition;
use super::line_color::LineColor;
use super::reverse_print::ReversePrint;

#[derive(Clone, Debug)]
pub struct GraphicBox {
    pub reverse_print: ReversePrint,
    pub position: LabelPosition,
    pub width: i32,
    pub height: i32,
    pub border_thickness: i32,
    pub corner_rounding: i32,
    pub line_color: LineColor,
}
