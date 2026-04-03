pub const COM1_PORT_RANGE: std::ops::RangeInclusive<u16> = 0x3F8..=0x3FF;

pub fn is_serial_port(port: u16) -> bool {
    COM1_PORT_RANGE.contains(&port)
}
