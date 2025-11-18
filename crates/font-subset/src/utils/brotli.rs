//! Shared `brotli`-related functionality.

use crate::alloc::{vec, Box, Vec};

#[derive(Debug)]
struct Slice<'a>(&'a [u8]);

impl brotli::CustomRead<()> for Slice<'_> {
    fn read(&mut self, data: &mut [u8]) -> Result<usize, ()> {
        let this_len = self.0.len();
        if this_len < data.len() {
            data[..this_len].copy_from_slice(self.0);
            self.0 = &[];
            Ok(this_len)
        } else {
            let (head, tail) = self.0.split_at(data.len());
            data.copy_from_slice(head);
            self.0 = tail;
            Ok(data.len())
        }
    }
}

#[derive(Default)]
struct Buffer(pub(crate) Vec<u8>);

impl brotli::CustomWrite<()> for Buffer {
    fn write(&mut self, data: &[u8]) -> Result<usize, ()> {
        self.0.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> Result<(), ()> {
        Ok(())
    }
}

#[derive(Debug, Clone)]
struct BoxedSlice<T>(Box<[T]>);

impl<T> Default for BoxedSlice<T> {
    fn default() -> Self {
        Self(Box::default())
    }
}

impl<T> brotli::SliceWrapper<T> for BoxedSlice<T> {
    fn slice(&self) -> &[T] {
        self.0.as_ref()
    }

    fn len(&self) -> usize {
        self.0.len()
    }
}

impl<T> brotli::SliceWrapperMut<T> for BoxedSlice<T> {
    fn slice_mut(&mut self) -> &mut [T] {
        self.0.as_mut()
    }
}

#[derive(Debug)]
struct GlobalAlloc;

impl<T: Clone + Default> brotli::enc::Allocator<T> for GlobalAlloc {
    type AllocatedMemory = BoxedSlice<T>;

    fn alloc_cell(&mut self, len: usize) -> Self::AllocatedMemory {
        BoxedSlice(vec![T::default(); len].into())
    }

    fn free_cell(&mut self, data: Self::AllocatedMemory) {
        drop(data);
    }
}

impl brotli::enc::BrotliAlloc for GlobalAlloc {}

pub(crate) fn compress(reader: &mut impl brotli::CustomRead<()>) -> Vec<u8> {
    let mut buffer = Buffer::default();
    brotli::BrotliCompressCustomIo(
        reader,
        &mut buffer,
        &mut [0_u8; 4_096],
        &mut [0_u8; 4_096],
        &::brotli::enc::BrotliEncoderParams::default(),
        GlobalAlloc,
        &mut |_, _, _, _| { /* do nothing */ },
        (),
    )
    .expect("Writing to Vec never fails");

    buffer.0
}

pub(crate) fn decompress(raw: &[u8]) -> Result<Vec<u8>, ()> {
    let mut buffer = Buffer::default();
    brotli::BrotliDecompressCustomIo(
        &mut Slice(raw),
        &mut buffer,
        &mut [0_u8; 4_096],
        &mut [0_u8; 4_096],
        GlobalAlloc,
        GlobalAlloc,
        GlobalAlloc,
        (),
    )
    .map(|()| buffer.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compression_roundtrip() {
        let bytes = b"Hello world! Hello world! Hello world!!!!!!!!!!!";
        let compressed = compress(&mut Slice(bytes));
        assert!(compressed.len() < bytes.len(), "{compressed:?}");
        let decompressed = decompress(&compressed).unwrap();
        assert_eq!(decompressed, bytes);
    }

    #[test]
    fn decompression_error() {
        decompress(&[]).unwrap_err();
    }
}
