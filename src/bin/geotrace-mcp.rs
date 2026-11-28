use std::env;
use std::error::Error;
use std::ffi::OsStr;

fn main() -> Result<(), Box<dyn Error>> {
    if env::args_os().nth(1).as_deref() == Some(OsStr::new("--version")) {
        println!("{} {}", gt_mcp::COMPANION_NAME, gt_mcp::VERSION);
        return Ok(());
    }
    gt_mcp::run()
}
