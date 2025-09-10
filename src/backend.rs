use std::time::Instant;

#[derive(Clone, Debug)]
pub struct Backend {
    pub host: String,
    pub port: u16,
    pub weight: usize,
    pub healthy: bool,
    pub last_checked: Option<Instant>,
}