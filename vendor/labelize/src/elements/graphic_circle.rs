use super::label_position::LabelPosition;
use super::line_color::LineColor;
use super::reverse_print::ReversePrint;

#[derive(Clone, Debug)]
pub struct GraphicCircle {
    pub reverse_print: ReversePrint,
    pub position: LabelPosition,
    pub circle_diameter: i32,
    pub border_thickness: i32,
    pub line_color: LineColor,
}
