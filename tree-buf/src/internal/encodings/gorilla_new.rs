use crate::prelude::*;
use num_traits::AsPrimitive;

// TODO: Remove warning
#[allow(dead_code)]
pub fn decompress<T: 'static + Copy>(_bytes: &[u8]) -> ReadResult<Vec<T>>
where
    f64: AsPrimitive<T>,
{
    todo!()
}

pub fn compress(data: impl Iterator<Item = f64>, bytes: &mut Vec<u8>) -> Result<ArrayTypeId, ()> {
    // FIXME: Verify current platform is little endian
    let mut data = data.map(f64::to_bits);

    let write = move |bits, count, capacity: &mut u8, buffer: &mut u64, bytes: &mut Vec<u8>| {
        if count <= *capacity {
            *buffer ^= bits << (*capacity - count);
            *capacity -= count;
        } else {
            let remainder = count - *capacity;
            *buffer ^= bits >> remainder;
            bytes.extend_from_slice(&buffer.to_le_bytes());
            *capacity = 64 - remainder;
            *buffer = bits << *capacity;
        }
    };

    let mut buffer = match data.next() {
        Some(first) => first,
        None => return Err(()),
    };

    let mut previous = buffer;
    let mut prev_xor = buffer;
    let mut capacity = 0;
    let capacity = &mut capacity;
    let buffer = &mut buffer;

    // TODO: This was written this way to match output the existing gorilla compressor, and may not
    // match the actual paper. Investigate.
    for value in data {
        let xored = previous ^ value;

        match xored {
            0 => write(0, 1, capacity, buffer, bytes),
            _ => {
                let lz = xored.leading_zeros().min(31) as u64;
                let tz = xored.trailing_zeros() as u64;
                let prev_lz = prev_xor.leading_zeros() as u64;
                let prev_tz = if prev_lz == 64 { 0 } else { prev_xor.trailing_zeros() as u64 };
                if lz >= prev_lz && tz >= prev_tz {
                    let meaningful_bits = xored >> prev_tz;
                    let meaningful_bit_count = 64 - prev_tz - prev_lz;

                    write(0b10, 2, capacity, buffer, bytes);
                    write(meaningful_bits, meaningful_bit_count as u8, capacity, buffer, bytes);
                } else {
                    let meaningful_bits = xored >> tz;
                    let meaningful_bit_count = 64 - tz - lz;

                    write(0b11, 2, capacity, buffer, bytes);
                    write(lz, 5, capacity, buffer, bytes);
                    write(meaningful_bit_count - 1, 6, capacity, buffer, bytes);
                    write(meaningful_bits, meaningful_bit_count as u8, capacity, buffer, bytes);
                }
            }
        };

        previous = value;
        prev_xor = xored;
    }

    // Add whatever is left
    let remaining = 64 - *capacity;
    let mut byte_count = remaining / 8;
    if byte_count * 8 != remaining {
        byte_count += 1;
    }
    let last = &(&buffer.to_le_bytes())[(8 - byte_count) as usize..];
    bytes.extend_from_slice(&last);
    bytes.push(remaining);

    Ok(ArrayTypeId::DoubleGorilla)
}
