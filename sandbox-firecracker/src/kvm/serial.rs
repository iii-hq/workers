pub const COM1_BASE: u16 = 0x3F8;
pub const COM1_IRQ: u32 = 4;
pub const COM1_PORT_RANGE: std::ops::RangeInclusive<u16> = 0x3F8..=0x3FF;

pub fn is_serial_port(port: u16) -> bool {
    COM1_PORT_RANGE.contains(&port)
}
