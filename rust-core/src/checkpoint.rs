//! Lossless four-lane XOR codec for ink checkpoint words. The host supplies
//! IEEE-754 bits, including signed zero and the pressure-presence lane.
fn encode(input: &[u8], output: &mut [u8]) -> Option<usize> {
    if input.len() % 8 != 0 || output.len() < input.len() / 8 * 9 { return None; }
    let mut previous = [0u64; 4];
    let mut written = 0;
    for (i, word) in input.chunks_exact(8).enumerate() {
        let bits = u64::from_le_bytes(word.try_into().ok()?);
        let delta = bits ^ previous[i % 4];
        previous[i % 4] = bits;
        let length = (64 - delta.leading_zeros() as usize).div_ceil(8);
        output[written] = length as u8;
        written += 1;
        output[written..written + length].copy_from_slice(&delta.to_le_bytes()[..length]);
        written += length;
    }
    Some(written)
}
fn decode(input: &[u8], output: &mut [u8]) -> Option<usize> {
    if output.len() % 8 != 0 { return None; }
    let mut previous = [0u64; 4];
    let mut read = 0;
    for (i, word) in output.chunks_exact_mut(8).enumerate() {
        let length = *input.get(read)? as usize;
        read += 1;
        if length > 8 { return None; }
        let mut bytes = [0u8; 8];
        bytes[..length].copy_from_slice(input.get(read..read.checked_add(length)?)?);
        read += length;
        let bits = u64::from_le_bytes(bytes) ^ previous[i % 4];
        previous[i % 4] = bits;
        word.copy_from_slice(&bits.to_le_bytes());
    }
    if read != input.len() { return None; }
    Some(output.len())
}
#[no_mangle]
pub extern "C" fn ink_checkpoint_version() -> u32 { 1 }
/// Buffers must be valid, separate allocations from the host ABI allocator.
#[no_mangle]
pub unsafe extern "C" fn ink_checkpoint_pack(src: *const u8, len: usize, dst: *mut u8, capacity: usize) -> i32 {
    if src.is_null() || dst.is_null() || len > 64 * 1024 * 1024 || capacity > 72 * 1024 * 1024 { return -1; }
    encode(std::slice::from_raw_parts(src,len),std::slice::from_raw_parts_mut(dst,capacity)).map_or(-1,|n|n as i32)
}
/// Returns -1 for malformed/truncated input; never accepts a partial stream.
#[no_mangle]
pub unsafe extern "C" fn ink_checkpoint_unpack(src: *const u8, len: usize, dst: *mut u8, capacity: usize) -> i32 {
    if src.is_null() || dst.is_null() || len > 72 * 1024 * 1024 || capacity > 64 * 1024 * 1024 { return -1; }
    decode(std::slice::from_raw_parts(src,len),std::slice::from_raw_parts_mut(dst,capacity)).map_or(-1,|n|n as i32)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_bits_roundtrip() {
        let words = [0u64,(-0.0f64).to_bits(),f64::INFINITY.to_bits(),f64::NAN.to_bits(),1, u64::MAX, 0x8000000000000001,42];
        let bytes:Vec<u8> = words.iter().flat_map(|w|w.to_le_bytes()).collect();
        let mut packed=vec![0;words.len()*9]; let len=encode(&bytes,&mut packed).unwrap();
        let mut decoded=vec![0;bytes.len()];assert_eq!(decode(&packed[..len],&mut decoded),Some(bytes.len()));assert_eq!(decoded,bytes);
        for end in 0..len {assert_eq!(decode(&packed[..end],&mut decoded),None);}
        packed[len-1]=9; // malformed length is separately checked below
        assert_eq!(decode(&[9],&mut [0;8]),None);
        assert_eq!(decode(&[0,0],&mut [0;8]),None);
    }
    #[test]
    fn repeated_points_compress() {
        let bytes:Vec<u8>=(0..400).flat_map(|_| 1f64.to_le_bytes()).collect();
        let mut packed=vec![0;3600];let len=encode(&bytes,&mut packed).unwrap();
        assert!(len < bytes.len()/4);
        let mut decoded=vec![0;bytes.len()];decode(&packed[..len],&mut decoded).unwrap();assert_eq!(decoded,bytes);
    }
}
