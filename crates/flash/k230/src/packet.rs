//! The burner's 60-byte packets (kburn.h, burner_uboot.cpp: "C:n" below is a
//! line of that file). A command and a reply are both whole packets: a
//! little-endian `cmd`, `result` and `data_size` (u16 each), then 54 bytes of
//! data. A reply's `cmd` is the command's with 0x8000 set.

/// burner_uboot.cpp:46 `KBUNR_USB_PKT_SIZE`.
pub const PACKET: usize = 60;
/// C:50-59: a 6-byte header, then this much data.
pub const DATA: usize = 54;
/// C:13: the reply flag on the command number.
pub const REPLY_FLAG: u16 = 0x8000;
/// C:14: stale replies skipped before a command fails.
pub const STALE_REPLIES: usize = 8;

/// C:16-33 `kburn_pkt_cmd`.
pub mod cmd {
    pub const NOP: u16 = 0x00;
    pub const REBOOT: u16 = 0x01;
    pub const PROBE: u16 = 0x10;
    pub const GET_INFO: u16 = 0x11;
    pub const ERASE: u16 = 0x20;
    pub const WRITE: u16 = 0x21;
    pub const WRITE_CHUNK: u16 = 0x22;
    pub const READ: u16 = 0x23;
    pub const READ_CHUNK: u16 = 0x24;
    pub const VERIFY: u16 = 0x25;
}

/// C:35-41 `kburn_pkt_result`.
pub const RESULT_OK: u16 = 1;
pub const RESULT_ERROR: u16 = 2;
pub const RESULT_MESSAGE: u16 = 0xff;

/// "Rbot", the reboot command's guard (C:`REBOOT_MARK`, sent as a u64).
pub const REBOOT_MARK: u64 = 0x5262_6f74;

/// A command packet: `data` goes after the header, the rest is zero.
pub fn packet(cmd: u16, data: &[u8]) -> [u8; PACKET] {
    debug_assert!(data.len() <= DATA, "a packet holds {DATA} bytes of data");
    let mut packet = [0u8; PACKET];
    packet[0..2].copy_from_slice(&cmd.to_le_bytes());
    packet[4..6].copy_from_slice(&(data.len() as u16).to_le_bytes());
    packet[6..6 + data.len()].copy_from_slice(data);
    packet
}

/// A little-endian u16 at `at`.
pub fn le16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

/// A little-endian u64 at `at`.
pub fn le64(bytes: &[u8], at: usize) -> u64 {
    let mut word = [0u8; 8];
    word.copy_from_slice(&bytes[at..at + 8]);
    u64::from_le_bytes(word)
}

/// Little-endian u64 words, the layout of every command's data.
pub fn words(values: &[u64]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_packet_is_sixty_bytes_with_the_data_after_the_header() {
        let bytes = packet(cmd::PROBE, &[2, 0xff]);
        assert_eq!(bytes.len(), PACKET);
        assert_eq!(&bytes[..6], &[0x10, 0, 0, 0, 2, 0]);
        assert_eq!(&bytes[6..8], &[2, 0xff]);
        assert!(bytes[8..].iter().all(|&byte| byte == 0));
    }
}
