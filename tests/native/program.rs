// Run through tools/test_program.sh. Only the NOR is simulated. tinfl is the
// reference C implementation linked into this host executable.
#[path = "../../targets/esp32p4/src/program.rs"]
mod program;

mod flash;

fn words(data: &[u32]) -> Vec<u8> {
    data.iter().flat_map(|word| word.to_le_bytes()).collect()
}
fn main() {
    use program::Download;
    use stub_core::protocol::data_checksum;
    let args: Vec<_> = std::env::args().collect();
    let plain = std::fs::read(&args[1]).unwrap();
    let compressed = std::fs::read(&args[2]).unwrap();
    // Exercise the target's real destination bounds and entry validation against
    // a private host mapping at the P4 download address.
    unsafe extern "C" {
        fn mmap(at: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, off: isize) -> *mut u8;
        fn munmap(at: *mut u8, len: usize) -> i32;
    }
    let addr = program::RAM_START;
    let region = unsafe { mmap(addr as *mut u8, 4096, 3, 0x100022, -1, 0) };
    assert_eq!(region, addr as *mut u8);
    let nor = flash::Nor(vec![0xff; 4096].into());
    assert!(Download::begin(5, &words(&[8, 1, 8, program::RAM_END - 4]), &nor).is_err());
    let mut ram = Download::begin(5, &words(&[8, 1, 8, addr]), &nor).unwrap();
    let mut segment = words(&[8, 0, 0, 0]);
    segment.extend_from_slice(b"RAM-test");
    ram.data(7, &segment, u32::from(data_checksum(b"RAM-test")), &nor)
        .unwrap();
    assert_eq!(
        unsafe { std::slice::from_raw_parts(region, 8) },
        b"RAM-test"
    );
    assert!(ram.finish(6, &words(&[0, addr + 8])).is_err());
    assert_eq!(ram.finish(6, &words(&[0, addr])).unwrap(), Some(addr));
    assert_eq!(unsafe { munmap(region, 4096) }, 0);
    for (kind, input) in [(2, plain.as_slice()), (0x10, compressed.as_slice())] {
        let nor = flash::Nor(vec![0x55; 131072].into());
        let block = 257;
        let begin = words(&[
            plain.len() as u32,
            input.len().div_ceil(block) as u32,
            block as u32,
            4096,
        ]);
        let mut state = Download::begin(kind, &begin, &nor).unwrap();
        assert!(state.finish(kind + 2, &words(&[1])).is_err());
        for (seq, data) in input.chunks(block).enumerate() {
            let mut packet = words(&[data.len() as u32, seq as u32, 0, 0]);
            packet.extend_from_slice(data);
            let sum = u32::from(data_checksum(data));
            assert_eq!(state.validate_data(kind + 1, &packet, sum ^ 1), Err(0xc1));
            state.data(kind + 1, &packet, sum, &nor).unwrap();
            assert!(state.validate_data(kind + 1, &packet, sum).is_err());
        }
        assert_eq!(state.finish(kind + 2, &words(&[1])).unwrap(), None);
        let bytes = nor.0.borrow();
        assert_eq!(&bytes[4096..4096 + plain.len()], plain);
        assert!(bytes[..4096].iter().all(|&b| b == 0x55));
        let after = 4096 + plain.len().div_ceil(4096) * 4096;
        assert!(bytes[after..].iter().all(|&b| b == 0x55));
    }
}
