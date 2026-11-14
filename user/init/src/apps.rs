//! Spousteni programu podle manifestu (`<program>.manifest.toml` v initfs):
//! kazdy program dostane vlastni job s rozpoctem z `[limits]` a relaci
//! site podle `[network]`. Program bez manifestu bezi v podrizenem jobu
//! bez limitu a sit nedostane.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use libos::initfs::Initfs;
use libos::job::{Job, pool};
use libos::spawn::StartHandle;
use libos::{Channel, Error, Process, Result, Startup, println};
use manifest::Manifest;
use netproto::Net;

pub struct App {
    pub process: Process,
    pub control: Channel,
    pub job: Job,
}

/// Manifest programu z initfs (`None` = program manifest nema).
pub fn manifest(fs: &Initfs, name: &str) -> Result<Option<Manifest>> {
    let Some(text) = fs.find(&format!("{name}.manifest.toml")) else {
        return Ok(None);
    };
    let text = core::str::from_utf8(text).map_err(|_| Error::Corrupted)?;
    match Manifest::parse(text) {
        Ok(m) => Ok(Some(m)),
        Err(e) => {
            println!("init: manifest {name}: {e}");
            Err(Error::InvalidArgs)
        }
    }
}

/// Podrizeny job s limity z manifestu.
pub fn job_for(m: Option<&Manifest>) -> Result<Job> {
    job_in(&libos::job::current()?, m)
}

/// Job s limity z manifestu pod jobem `parent` (napr. job relace).
pub fn job_in(parent: &Job, m: Option<&Manifest>) -> Result<Job> {
    let job = Job::create(parent)?;
    // Aplikace si samy nedaji vic nez Normal; kompozitor je pri obsluze
    // vstupu zvedne zpravou (dedeni QoS pres IPC).
    libos::sched::set_job_max(&job, libos::sched::Qos::Normal)?;
    if let Some(m) = m {
        if let Some(v) = m.limits.memory {
            job.set_limit(pool::USER, v)?;
        }
        if let Some(v) = m.limits.kernel_memory {
            job.set_limit(pool::KERNEL, v)?;
        }
        // Opravneni JIT jen podle manifestu (rodic musi smet udelovat).
        if m.jit {
            job.set_jit(libos::abi::job::jit::RUN)?;
        }
    }
    Ok(job)
}

/// Spusti program z initfs v jeho jobu.
pub fn spawn(startup: &Startup, name: &str, handles: Vec<StartHandle>) -> Result<App> {
    spawn_in(startup, name, handles, None)
}

/// Jako [`spawn`], job programu pod jobem `parent` (`None` = job initu).
pub fn spawn_in(startup: &Startup, name: &str, handles: Vec<StartHandle>, parent: Option<&Job>) -> Result<App> {
    let fs = Initfs::open(startup)?;
    let image = fs.find(name).ok_or(Error::NotFound)?;
    let manifest = manifest(&fs, name)?;
    let job = match parent {
        Some(p) => job_in(p, manifest.as_ref())?,
        None => job_for(manifest.as_ref())?,
    };
    let mut opts = libos::spawn::Options::default();
    if let Some(s) = manifest.as_ref().and_then(|m| m.limits.stack) {
        opts.stack_size = s;
    }
    let (process, control) = libos::spawn::spawn_with(&job, name, image, handles, opts)?;
    if let Some(m) = &manifest {
        let mem = |v: Option<u64>| v.map_or(String::from("bez limitu"), |b| format!("{} MiB", b >> 20));
        println!(
            "init: {name} ({}) v jobu: pamet {}, pamet jadra {}",
            m.id,
            mem(m.limits.memory),
            mem(m.limits.kernel_memory)
        );
    }
    Ok(App { process, control, job })
}

/// Relace site pro aplikaci podle manifestu (`None` = manifest sit nechce).
pub fn net_session(root: &Net, m: &Manifest) -> Result<Option<Net>> {
    match &m.network {
        None => Ok(None),
        Some(r) => root.restrict_with(r).map(Some),
    }
}
