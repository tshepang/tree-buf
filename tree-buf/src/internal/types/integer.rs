use crate::internal::encodings::compress;
use crate::internal::encodings::varint::*;
use crate::prelude::*;
use num_traits::{AsPrimitive, Bounded};
use simple_16::compress as compress_simple_16;
use std::any::TypeId;
use std::convert::{TryFrom, TryInto};
use std::mem::transmute;
use std::vec::IntoIter;

#[derive(Copy, Clone)]
struct U0;

impl Bounded for U0 {
    fn min_value() -> Self {
        U0
    }
    fn max_value() -> Self {
        U0
    }
}

fn write_u0<T, O: EncodeOptions>(_data: &[T], _max: T, _stream: &mut WriterStream<'_, O>) -> ArrayTypeId {
    unreachable!();
}

macro_rules! impl_lowerable {
    ($Ty:ty, $fn:ident, $Lty:ty, $lfn:ident, ($($lower:ty),*), ($($compressions:ty),+)) => {
        impl TryFrom<$Ty> for U0 {
            type Error=();
            fn try_from(_value: $Ty) -> Result<U0, Self::Error> {
                Err(())
            }
        }
        impl TryFrom<U0> for $Ty {
            type Error=();
            fn try_from(_value: U0) -> Result<$Ty, Self::Error> {
                Err(())
            }
        }
        impl AsPrimitive<U0> for $Ty {
            fn as_(self) -> U0 {
                unreachable!()
            }
        }

        #[cfg(feature = "write")]
        impl Writable for $Ty {
            type WriterArray = Vec<$Ty>;
            fn write_root<O: EncodeOptions>(&self, stream: &mut WriterStream<'_, O>) -> RootTypeId {
                write_root_uint(*self as u64, stream.bytes)
            }
        }

        #[cfg(feature = "write")]
        impl WriterArray<$Ty> for Vec<$Ty> {
            fn buffer<'a, 'b: 'a>(&'a mut self, value: &'b $Ty) {
                self.push(*value);
            }
            fn flush<O: EncodeOptions>(self, stream: &mut WriterStream<'_, O>) -> ArrayTypeId {
                profile!("WriterArray::flush");
                let max = self.iter().max();
                if let Some(max) = max {
                    // TODO: (Performance) Use second-stack
                    // Lower to bool if possible. This is especially nice for enums
                    // with 2 variants.
                    // TODO: Lowering to bool works in all tests, but not benchmarks
                    if *max < 2 {
                        let bools = self.iter().map(|i| *i == 1).collect::<Vec<_>>();
                        bools.flush(stream)
                    } else {
                        $fn(&self, *max, stream)
                    }
                } else {
                    ArrayTypeId::Void
                }
            }
        }

        #[cfg(feature = "read")]
        impl Readable for $Ty {
            type ReaderArray = IntoIter<$Ty>;
            fn read(sticks: DynRootBranch<'_>, _options: &impl DecodeOptions) -> ReadResult<Self> {
                profile!("Readable::read");
                match sticks {
                    DynRootBranch::Integer(root_int) => {
                        match root_int {
                            RootInteger::U(v) => v.try_into().map_err(|_| ReadError::SchemaMismatch),
                            _ => Err(ReadError::SchemaMismatch),
                        }
                    }
                    _ => Err(ReadError::SchemaMismatch),
                }
            }
        }

        #[cfg(feature = "read")]
        impl InfallibleReaderArray for IntoIter<$Ty> {
            type Read = $Ty;
            fn new_infallible(sticks: DynArrayBranch<'_>, options: &impl DecodeOptions) -> ReadResult<Self> {
                profile!(Self::Read, "ReaderArray::new");

                match sticks {
                    // TODO: Support eg: delta/zigzag
                    DynArrayBranch::Integer(array_int) => {
                        let ArrayInteger { bytes, encoding } = array_int;
                        match encoding {
                            ArrayIntegerEncoding::PrefixVarInt => {
                                #[cfg(feature="profile")]
                                let _g = flame::start_guard("PrefixVarInt");

                                let v: Vec<$Ty> = read_all(
                                        &bytes,
                                        |bytes, offset| {
                                            let r: $Ty = decode_prefix_varint(bytes, offset)?.try_into().map_err(|_| ReadError::SchemaMismatch)?;
                                            Ok(r)
                                        }
                                )?;
                                Ok(v.into_iter())
                            }
                            ArrayIntegerEncoding::Simple16 => {
                                #[cfg(feature="profile")]
                                let _g = flame::start_guard("Simple16");

                                let mut v = Vec::new();
                                simple_16::decompress(&bytes, &mut v).map_err(|_| ReadError::InvalidFormat)?;
                                let result: Result<Vec<_>, _> = v.into_iter().map(TryInto::<$Ty>::try_into).collect();
                                let v = result.map_err(|_| ReadError::SchemaMismatch)?;
                                Ok(v.into_iter())
                            },
                            ArrayIntegerEncoding::U8 => {
                                #[cfg(feature="profile")]
                                let _g = flame::start_guard("U8");

                                let v: Vec<$Ty> = bytes.iter().map(|&b| b.into()).collect();
                                Ok(v.into_iter())
                            }
                        }
                    },
                    DynArrayBranch::RLE { runs, values } => {
                        let rle = RleIterator::new(runs, values, options, |values| Self::new_infallible(values, options))?;
                        let all = rle.collect::<Vec<_>>();
                        Ok(all.into_iter())
                    },
                    // FIXME: This fixes a particular test.
                    // It is unclear if this is canon.
                    // See also: 84d15459-35e4-4f04-896f-0f4ea9ce52a9
                    // TODO: Also apply this to other types
                    DynArrayBranch::Void => {
                        Ok(Vec::new().into_iter())
                    }
                    other => {
                        let bools = <IntoIter<bool> as InfallibleReaderArray>::new_infallible(other, options)?;
                        let mapped = bools.map(|i| if i {1} else {0}).collect::<Vec<_>>();
                        Ok(mapped.into_iter())
                    },
                }
            }
            fn read_next_infallible(&mut self) -> Self::Read {
                self.next().unwrap_or_default()
            }
        }

        #[cfg(feature = "write")]
        fn $fn<O: EncodeOptions, T: Copy + std::fmt::Debug + AsPrimitive<$Ty> + AsPrimitive<U0> + AsPrimitive<u8> + AsPrimitive<$Lty> $(+ AsPrimitive<$lower>),*>
            (data: &[T], max: T, stream: &mut WriterStream<'_, O>) -> ArrayTypeId {
            profile!($Ty, "lowering_fn");

            // TODO: (Performance) When getting ranges, use SIMD
            let lower_max: Result<$Ty, _> = <$Lty as Bounded>::max_value().try_into();

            if let Ok(lower_max) = lower_max {
                if lower_max >= max.as_() {
                    return $lfn(data, max, stream)
                }
            }

            fn write_inner<O: EncodeOptions>(data: &[$Ty], stream: &mut WriterStream<'_, O>) -> ArrayTypeId {
                profile!(&[$Ty], "write_inner");

                let compressors = (
                    $(<$compressions>::new(),)+
                    RLE::new(($(<$compressions>::new(),)+))
                );
                compress(data, stream, &compressors)
            }

            // Convert data to as<T>, using a transmute if that's already correct
            if TypeId::of::<$Ty>() == TypeId::of::<T>() {
                // Safety - this is a unit conversion.
                let data = unsafe { transmute(data) };
                write_inner(data, stream)
            } else {
                // TODO: (Performance) Use second-stack
                let mut v = Vec::new();
                for item in data.iter() {
                    v.push(item.as_());
                }
                write_inner(&v, stream)
            }
        }
    };
}

// TODO: This does all kinds of silly things. Eg: Perhaps we have u32 and simple16 is best.
// This may downcast to u16 then back up to u32. I'm afraid the final result is just going to
// be a bunch of hairy special code for each type with no generality.
//
// Broadly we only want to downcast if it allows for some other kind of compressor to be used.

// Type, array writer, next lower, next lower writer, non-inferred lowers
impl_lowerable!(u64, write_u64, u32, write_u32, (u16), (PrefixVarIntCompressor));
impl_lowerable!(u32, write_u32, u16, write_u16, (), (Simple16Compressor, PrefixVarIntCompressor)); // TODO: Consider replacing PrefixVarInt at this level with Fixed.
impl_lowerable!(u16, write_u16, u8, write_u8, (), (Simple16Compressor, PrefixVarIntCompressor));
impl_lowerable!(u8, write_u8, U0, write_u0, (), (Simple16Compressor, BytesCompressor));

#[cfg(feature = "write")]
fn write_root_uint(value: u64, bytes: &mut Vec<u8>) -> RootTypeId {
    let le = value.to_le_bytes();
    match value {
        0 => RootTypeId::Zero,
        1 => RootTypeId::One,
        2..=255 => {
            bytes.push(le[0]);
            RootTypeId::IntU8
        }
        256..=65535 => {
            bytes.extend_from_slice(&le[..2]);
            RootTypeId::IntU16
        }
        65536..=16777215 => {
            bytes.extend_from_slice(&le[..3]);
            RootTypeId::IntU24
        }
        16777216..=4294967295 => {
            bytes.extend_from_slice(&le[..4]);
            RootTypeId::IntU32
        }
        4294967296..=1099511627775 => {
            bytes.extend_from_slice(&le[..5]);
            RootTypeId::IntU40
        }
        1099511627776..=281474976710655 => {
            bytes.extend_from_slice(&le[..6]);
            RootTypeId::IntU48
        }
        281474976710656..=72057594037927936 => {
            bytes.extend_from_slice(&le[..7]);
            RootTypeId::IntU56
        }
        _ => {
            bytes.extend_from_slice(&le);
            RootTypeId::IntU64
        }
    }
}

struct PrefixVarIntCompressor;

impl PrefixVarIntCompressor {
    pub fn new() -> Self {
        Self
    }
}

impl<T: Into<u64> + Copy> Compressor<T> for PrefixVarIntCompressor {
    fn fast_size_for(&self, data: &[T]) -> Option<usize> {
        profile!("Compressor::fast_size_for");
        let mut size = 0;
        for item in data {
            size += size_for_varint((*item).into());
        }
        Some(size)
    }
    fn compress<O: EncodeOptions>(&self, data: &[T], stream: &mut WriterStream<'_, O>) -> Result<ArrayTypeId, ()> {
        profile!("compress");
        stream.write_with_len(|stream| {
            for item in data {
                encode_prefix_varint((*item).into(), &mut stream.bytes);
            }
        });
        Ok(ArrayTypeId::IntPrefixVar)
    }
}

struct Simple16Compressor;

impl Simple16Compressor {
    pub fn new() -> Self {
        Self
    }
}

impl<T: Into<u32> + Copy> Compressor<T> for Simple16Compressor {
    fn compress<O: EncodeOptions>(&self, data: &[T], stream: &mut WriterStream<'_, O>) -> Result<ArrayTypeId, ()> {
        profile!("compress");
        // TODO: (Performance) Use second-stack.
        // TODO: (Performance) This just copies to another Vec in the case where T is u32

        let v = {
            #[cfg(feature = "profile")]
            flame::start_guard("Needless copy to u32");
            let mut v = Vec::new();
            for item in data {
                let item = *item;
                let item = item.try_into().map_err(|_| ())?;
                v.push(item);
            }
            v
        };

        stream.write_with_len(|stream| compress_simple_16(&v, stream.bytes)).map_err(|_| ())?;

        Ok(ArrayTypeId::IntSimple16)
    }
}

struct BytesCompressor;
impl BytesCompressor {
    pub fn new() -> Self {
        Self
    }
}

impl Compressor<u8> for BytesCompressor {
    fn compress<O: EncodeOptions>(&self, data: &[u8], stream: &mut WriterStream<'_, O>) -> Result<ArrayTypeId, ()> {
        profile!("compress");
        stream.write_with_len(|stream| stream.bytes.extend_from_slice(data));
        Ok(ArrayTypeId::U8)
    }
    fn fast_size_for(&self, data: &[u8]) -> Option<usize> {
        Some(data.len())
    }
}

// TODO: Bitpacking https://crates.io/crates/bitpacking
// TODO: Mayda https://crates.io/crates/mayda
// TODO: https://lemire.me/blog/2012/09/12/fast-integer-compression-decoding-billions-of-integers-per-second/
