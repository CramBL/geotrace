//! Regression tests of hdf5-pure, the library that reads and writes the HDF5
//! layer of a `.gtd` file.

#![expect(
    clippy::panic_in_result_fn,
    reason = "test functions mix ? propagation with assert! - both are correct in test code"
)]

use hdf5_pure::{File, FileBuilder};

/// hdf5-pure 0.5.0 and earlier failed to read a dataset with a paged Fixed
/// Array chunk index. The library writes that index for a dataset of more than
/// 1024 chunks.
#[test]
fn a_dataset_of_more_than_1024_chunks_reads_back() -> Result<(), Box<dyn std::error::Error>> {
    const CHUNK_COUNT: usize = 1025;
    let data: Vec<f64> = (0..CHUNK_COUNT).map(|i| i as f64).collect();

    let mut fb = FileBuilder::new();
    let mut grp = fb.create_group("data");
    grp.create_dataset("values")
        .with_f64_data(&data)
        .with_shape(&[CHUNK_COUNT as u64])
        .with_chunks(&[1])
        .with_deflate(6);
    fb.add_group(grp.finish());

    let file = File::from_bytes(fb.finish()?)?;
    let read_back = file.group("data")?.dataset("values")?.read_f64()?;
    assert_eq!(read_back, data);
    Ok(())
}
