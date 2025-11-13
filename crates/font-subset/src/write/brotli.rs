//! Brotli compression support.

use core::ops;

use super::FontWriter;

struct TableDataReader<'a> {
    writer: &'a FontWriter,
    data_offset: u32,
    table_idx: usize,
    pos_in_table: usize,
}

impl<'a> TableDataReader<'a> {
    fn new(writer: &'a FontWriter) -> Self {
        debug_assert!(
            writer.tables.windows(2).all(|window| {
                let [prev, next] = window else {
                    unreachable!();
                };
                prev.offset + prev.length <= next.offset
            }),
            "table records need to be ordered by offsets"
        );
        let data_offset = writer.tables.first().map_or(0, |record| record.offset);

        Self {
            writer,
            data_offset,
            table_idx: 0,
            pos_in_table: 0,
        }
    }

    fn read_chunk<'data>(
        &self,
        range: ops::Range<usize>,
        data: &'data mut [u8],
    ) -> (usize, Option<&'data mut [u8]>) {
        let remaining = &self.writer.table_data[range];
        if remaining.len() < data.len() {
            let (head, tail) = data.split_at_mut(remaining.len());
            head.copy_from_slice(remaining);
            // Continue reading from the next table
            (remaining.len(), Some(tail))
        } else {
            data.copy_from_slice(&remaining[..data.len()]);
            (data.len(), None)
        }
    }
}

impl brotli::CustomRead<()> for TableDataReader<'_> {
    fn read(&mut self, mut data: &mut [u8]) -> Result<usize, ()> {
        let mut total_read = 0;
        loop {
            let Some(table) = self.writer.tables.get(self.table_idx) else {
                return Ok(total_read); // nothing left to read
            };

            let adjusted_offset = (table.offset - self.data_offset) as usize;
            let start_offset = adjusted_offset + self.pos_in_table;
            let end_offset = adjusted_offset + table.length as usize;
            let (read, remaining_data) = self.read_chunk(start_offset..end_offset, data);
            total_read += read;

            if let Some(remaining_data) = remaining_data {
                // Move to the next table
                self.table_idx += 1;
                self.pos_in_table = 0;
                data = remaining_data;
            } else {
                // Run out of the output buffer
                self.pos_in_table += read;
                debug_assert!(self.pos_in_table <= table.length as usize);
                return Ok(total_read);
            }
        }
    }
}

#[derive(Default)]
struct Buffer(Vec<u8>);

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

impl FontWriter {
    pub(super) fn compress_data(&self) -> Vec<u8> {
        let mut buffer = Buffer::default();
        ::brotli::BrotliCompressCustomIo(
            &mut TableDataReader::new(self),
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
}

#[cfg(test)]
mod tests {
    use std::fs;

    use brotli::CustomRead;
    use test_casing::test_casing;

    use super::*;
    use crate::{Font, FontSubset};

    #[test_casing(5, [1, 10, 100, 1000, 100_000])]
    fn table_data_reader_works_as_expected(chunk_size: usize) {
        let font_bytes = fs::read("examples/FiraMono-Regular.ttf").unwrap();
        let font = Font::new(&font_bytes).unwrap();
        let chars = (' '..='~').collect();
        let subset = FontSubset::new(font, &chars).unwrap();
        let writer = subset.to_writer();

        let mut data_reader = TableDataReader::new(&writer);
        let mut buffer = vec![0; 100_000];

        let read = buffer
            .chunks_mut(chunk_size)
            .map(|chunk| data_reader.read(chunk).unwrap())
            .sum::<usize>();
        let expected_read = writer
            .tables
            .iter()
            .map(|record| record.length as usize)
            .sum::<usize>();
        assert_eq!(read, expected_read);

        let mut pos = 0;
        for record in &writer.tables {
            let offset = record.offset as usize;
            let len = record.length as usize;
            assert_eq!(
                writer.table_data[offset..offset + len],
                buffer[pos..pos + len]
            );
            pos += len;
        }
    }
}
