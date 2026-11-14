//! Backend GPU kompozitoru (`backend gpu`, `user/compd/src/gpu.rs`,
//! `libs/vkcomp`): stejne pixely obrazovky jako CPU (mrizka bodu pres
//! inspekci). Bez Venusu (`NOT_SUPPORTED`) se test preskoci; Venus je jen
//! v `cargo xtask test --qemu-wsl --gpu venus`.

use alloc::vec::Vec;

use libos::{Startup, println};

use super::gui::{State, pixel, settle, with};
use super::{TestResult, fail};

pub fn gpu_backend(_: &Startup) -> TestResult {
    with(|s| {
        let (w, h) = s.gui.size;
        settle(&s.inspect)?;
        let points: Vec<(i32, i32)> =
            (0..9).flat_map(|i| (0..7).map(move |j| ((w as i32 * (2 * i + 1)) / 18, (h as i32 * (2 * j + 1)) / 14))).collect();
        let sample = |s: &State| -> Result<Vec<u32>, alloc::string::String> {
            points.iter().map(|&(x, y)| pixel(&s.inspect, x, y)).collect()
        };
        let cpu = sample(s)?;
        // Render uzel od gpud (bez nej, napr. bez virtio-gpu 3D, NOT_SUPPORTED).
        let render = match crate::render::open_render() {
            Ok(ch) => alloc::vec![ch.into_handle()],
            Err(_) => Vec::new(),
        };
        match s.gui.command(b"backend gpu", render) {
            Ok(_) => {}
            Err(libos::Error::NotSupported) => {
                println!("init: test gui: GPU backend neni (bez Venusu), preskoceno");
                return Ok(());
            }
            Err(e) => return Err(fail("backend gpu", e)),
        }
        settle(&s.inspect)?;
        let gpu = sample(s);
        let back = s.gui.command(b"backend cpu", Vec::new());
        let gpu = gpu?;
        back.map_err(|e| fail("backend cpu", e))?;
        // Displej alfu ignoruje: porovnava se RGB.
        let diff: Vec<usize> = (0..cpu.len()).filter(|&i| (cpu[i] ^ gpu[i]) & 0x00FF_FFFF != 0).collect();
        if let Some(&i) = diff.first() {
            return Err(alloc::format!(
                "{} z {} bodu se lisi, prvni {:?}: cpu {:#010x} gpu {:#010x}",
                diff.len(),
                cpu.len(),
                points[i],
                cpu[i],
                gpu[i]
            ));
        }
        println!("init: test gui: GPU backend: {} bodu obrazovky stejne jako CPU", cpu.len());
        Ok(())
    })
}
