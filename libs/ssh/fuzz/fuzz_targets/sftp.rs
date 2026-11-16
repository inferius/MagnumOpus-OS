//! Server SFTP: libovolne bajty kanalu (po INIT).
#![no_main]

use libfuzzer_sys::fuzz_target;
use ssh::sftp::{Attrs, Sftp, SftpFs};

struct NullFs;

impl SftpFs for NullFs {
    fn stat(&mut self, _: &str) -> Result<Attrs, abi::Error> {
        Ok(Attrs::default())
    }
    fn list(&mut self, _: &str) -> Result<Vec<(String, Attrs)>, abi::Error> {
        Ok(vec![(String::from("a"), Attrs::default())])
    }
    fn read(&mut self, _: &str, _: u64, len: usize) -> Result<Vec<u8>, abi::Error> {
        Ok(vec![1; len.min(16)])
    }
    fn write(&mut self, _: &str, _: u64, _: &[u8]) -> Result<(), abi::Error> {
        Ok(())
    }
    fn create(&mut self, _: &str) -> Result<(), abi::Error> {
        Ok(())
    }
    fn truncate(&mut self, _: &str, _: u64) -> Result<(), abi::Error> {
        Ok(())
    }
    fn mkdir(&mut self, _: &str) -> Result<(), abi::Error> {
        Ok(())
    }
    fn remove(&mut self, _: &str) -> Result<(), abi::Error> {
        Ok(())
    }
    fn rename(&mut self, _: &str, _: &str) -> Result<(), abi::Error> {
        Ok(())
    }
}

fuzz_target!(|data: &[u8]| {
    let mut s = Sftp::new("/home/a");
    let mut fs = NullFs;
    let _ = s.input(&[0, 0, 0, 5, 1, 0, 0, 0, 3], &mut fs);
    let _ = s.input(data, &mut fs);
});
