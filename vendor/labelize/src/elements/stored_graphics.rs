#[derive(Clone, Debug)]
pub struct StoredGraphics {
    pub data: Vec<u8>,
    pub total_bytes: i32,
    pub row_bytes: i32,
}
