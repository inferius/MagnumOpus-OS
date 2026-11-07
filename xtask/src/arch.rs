//! Cilove architektury: Rust cile, jmeno zavadece v ESP, QEMU a firmware.

use std::env;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Popis jedne architektury systemu.
pub struct Arch {
    /// Jmeno pro `--arch` a nazvy souboru (`os-<name>-...img`).
    pub name: &'static str,
    /// Rust cil jadra (bez FPU/SIMD, viz `kernel/src/arch/api`).
    pub kernel_target: &'static str,
    /// Rust cil UEFI loaderu.
    pub loader_target: &'static str,
    /// Rust cil programu v userspace. Zatim bez FPU (soft-float) na obou
    /// architekturach; jadro presto uklada stav FPU pri prepnuti vlaken,
    /// takze programy s FPU (napr. jazyky s vlastnim cilem) fungovat budou.
    pub user_target: &'static str,
    /// Cesta loaderu v ESP, kterou firmware spusti sam (vychozi zavadec).
    pub boot_file: &'static str,
    /// Spustitelny soubor QEMU (bez pripony).
    pub qemu: &'static str,
    /// Dvojice (firmware code, sablona promennych) v adresari `share` QEMU.
    pub firmware: (&'static str, &'static str),
    /// Hodnoty `--iommu` (prvni je vychozi).
    pub iommus: &'static [&'static str],
    /// Navratovy kod QEMU pri uspechu testu (viz `arch::qemu_exit` v jadru).
    pub test_success: i32,
    /// Jak dlouho smi testy v QEMU bezet (aarch64 bezi v emulaci TCG).
    pub test_timeout: Duration,
}

pub const X86_64: Arch = Arch {
    name: "x86_64",
    kernel_target: "x86_64-unknown-none",
    loader_target: "x86_64-unknown-uefi",
    user_target: "x86_64-unknown-none",
    boot_file: "EFI/BOOT/BOOTX64.EFI",
    qemu: "qemu-system-x86_64",
    firmware: ("edk2-x86_64-code.fd", "edk2-i386-vars.fd"),
    iommus: &["intel", "amd", "none"],
    // isa-debug-exit vraci (v << 1) | 1, jadro zapisuje v = 0x10.
    test_success: 33,
    test_timeout: Duration::from_secs(120),
};

pub const AARCH64: Arch = Arch {
    name: "aarch64",
    kernel_target: "aarch64-unknown-none-softfloat",
    loader_target: "aarch64-unknown-uefi",
    user_target: "aarch64-unknown-none-softfloat",
    boot_file: "EFI/BOOT/BOOTAA64.EFI",
    qemu: "qemu-system-aarch64",
    firmware: ("edk2-aarch64-code.fd", "edk2-arm-vars.fd"),
    iommus: &["smmu", "none"],
    // Semihosting SYS_EXIT: QEMU skonci s kodem, ktery jadro preda.
    test_success: 0,
    test_timeout: Duration::from_secs(600),
};

pub const ALL: &[&Arch] = &[&X86_64, &AARCH64];

pub fn by_name(name: &str) -> Option<&'static Arch> {
    ALL.iter().copied().find(|a| a.name == name)
}

impl Arch {
    /// Najde QEMU: promenna `QEMU_<ARCH>` (nebo `QEMU` pro x86_64), PATH,
    /// bezna instalacni mista.
    pub fn find_qemu(&self) -> Result<PathBuf, String> {
        let var = format!("QEMU_{}", self.name.to_uppercase());
        if let Some(path) = env::var_os(&var) {
            return Ok(path.into());
        }
        if self.name == "x86_64"
            && let Some(path) = env::var_os("QEMU")
        {
            return Ok(path.into());
        }
        let exe = if cfg!(windows) {
            format!("{}.exe", self.qemu)
        } else {
            self.qemu.to_string()
        };
        if let Some(paths) = env::var_os("PATH") {
            for dir in env::split_paths(&paths) {
                let candidate = dir.join(&exe);
                if candidate.is_file() {
                    return Ok(candidate);
                }
            }
        }
        [
            r"C:\Program Files\qemu",
            "/opt/homebrew/bin",
            "/usr/local/bin",
        ]
        .iter()
        .map(|dir| Path::new(dir).join(&exe))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            format!(
                "{} nenalezeno (nastav {var} nebo ho pridej do PATH)",
                self.qemu
            )
        })
    }

    /// Najde firmware UEFI (code + sablona vars) ze stejne distribuce.
    pub fn find_firmware(&self, qemu: &Path) -> Result<(PathBuf, PathBuf), String> {
        let prefix = self.name.to_uppercase();
        if let (Some(code), Some(vars)) = (
            env::var_os(format!("{prefix}_FW_CODE")),
            env::var_os(format!("{prefix}_FW_VARS")),
        ) {
            return Ok((code.into(), vars.into()));
        }
        if self.name == "x86_64"
            && let (Some(code), Some(vars)) = (env::var_os("OVMF_CODE"), env::var_os("OVMF_VARS"))
        {
            return Ok((code.into(), vars.into()));
        }
        let (code, vars) = self.firmware;
        let mut candidates: Vec<(PathBuf, PathBuf)> = Vec::new();
        if let Some(bin_dir) = qemu.parent() {
            for share in [bin_dir.join("share"), bin_dir.join("../share/qemu")] {
                candidates.push((share.join(code), share.join(vars)));
            }
        }
        let system: &[(&str, &str)] = if self.name == "x86_64" {
            &[
                (
                    "/usr/share/OVMF/OVMF_CODE_4M.fd",
                    "/usr/share/OVMF/OVMF_VARS_4M.fd",
                ),
                (
                    "/usr/share/OVMF/OVMF_CODE.fd",
                    "/usr/share/OVMF/OVMF_VARS.fd",
                ),
                (
                    "/usr/share/edk2/x64/OVMF_CODE.4m.fd",
                    "/usr/share/edk2/x64/OVMF_VARS.4m.fd",
                ),
                (
                    "/usr/share/edk2/ovmf/OVMF_CODE.fd",
                    "/usr/share/edk2/ovmf/OVMF_VARS.fd",
                ),
                (
                    "/usr/share/qemu/edk2-x86_64-code.fd",
                    "/usr/share/qemu/edk2-i386-vars.fd",
                ),
            ]
        } else {
            &[
                (
                    "/usr/share/AAVMF/AAVMF_CODE.fd",
                    "/usr/share/AAVMF/AAVMF_VARS.fd",
                ),
                (
                    "/usr/share/qemu/edk2-aarch64-code.fd",
                    "/usr/share/qemu/edk2-arm-vars.fd",
                ),
                (
                    "/usr/share/edk2/aarch64/QEMU_EFI-pflash.raw",
                    "/usr/share/edk2/aarch64/vars-template-pflash.raw",
                ),
            ]
        };
        for (c, v) in system {
            candidates.push((c.into(), v.into()));
        }
        candidates
            .into_iter()
            .find(|(c, v)| c.is_file() && v.is_file())
            .ok_or_else(|| {
                format!(
                    "firmware UEFI pro {} nenalezen (nastav {prefix}_FW_CODE a {prefix}_FW_VARS)",
                    self.name
                )
            })
    }
}
