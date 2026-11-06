#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextAlignment {
    #[default]
    Left = 0,
    Right = 1,
    Justified = 2,
    Center = 3,
}
