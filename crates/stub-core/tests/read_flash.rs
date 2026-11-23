use stub_core::protocol::{FlashRead, FlashReadIo, ReadError, ReadScratch};

#[derive(Default)]
struct Sim {
    reads: Vec<(u32, usize)>,
    frames: Vec<Vec<u8>>,
    hashed: Vec<u8>,
    acknowledgements: std::collections::VecDeque<u32>,
}

impl FlashReadIo for Sim {
    fn read_aligned(&mut self, address: u32, output: &mut [u8]) -> Result<(), ReadError> {
        assert_eq!(
            (address as usize | output.len() | output.as_ptr() as usize) & 3,
            0
        );
        self.reads.push((address, output.len()));
        for (index, byte) in output.iter_mut().enumerate() {
            *byte = address.wrapping_add(index as u32) as u8;
        }
        Ok(())
    }

    fn send_frame(&mut self, payload: &[u8]) -> Result<(), ReadError> {
        self.frames.push(payload.to_vec());
        Ok(())
    }

    fn receive_ack(&mut self) -> Result<u32, ReadError> {
        self.acknowledgements
            .pop_front()
            .ok_or(ReadError::Transport)
    }

    fn begin_digest(&mut self) {
        self.hashed.clear();
    }

    fn update_digest(&mut self, bytes: &[u8]) {
        self.hashed.extend_from_slice(bytes);
    }

    fn finish_digest(&mut self) -> [u8; 16] {
        [0xab; 16]
    }
}

fn request() -> FlashRead {
    let payload: Vec<u8> = [3u32, 7, 5, 64]
        .into_iter()
        .flat_map(u32::to_le_bytes)
        .collect();
    FlashRead::parse(&payload, 16).unwrap()
}

#[test]
fn unaligned_read_and_short_final_packet() {
    let mut sim = Sim::default();
    sim.acknowledgements.extend([5, 7]);
    request()
        .transfer(&mut sim, &mut ReadScratch([0; 4100]))
        .unwrap();
    assert_eq!(sim.reads, [(0, 8), (8, 4)]);
    assert_eq!(
        sim.frames,
        [vec![3, 4, 5, 6, 7], vec![8, 9], vec![0xab; 16]]
    );
    assert_eq!(sim.hashed, [3, 4, 5, 6, 7, 8, 9]);
}

#[test]
fn wrong_ack_aborts_before_second_read_and_digest() {
    let mut sim = Sim::default();
    sim.acknowledgements.push_back(6);
    let result = request().transfer(&mut sim, &mut ReadScratch([0; 4100]));
    assert_eq!(result, Err(ReadError::UnexpectedAck));
    assert_eq!(sim.reads.len(), 1);
    assert_eq!(sim.frames.len(), 1);
}
