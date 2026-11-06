#[derive(Clone, Debug, Default)]
pub struct ReversePrint {
    pub value: bool,
}

impl ReversePrint {
    pub fn is_reverse_print(&self) -> bool {
        self.value
    }
}
