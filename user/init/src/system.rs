//! Start systemovych sluzeb: sluzba casu, spravce zarizeni a souborovy
//! system.

use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use blockproto::BlockClient;
use fsproto::FsClient;
use libos::abi::bootstrap::kind;
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Process, Result, Rights, Startup, println, time};

/// Bezici spravce zarizeni, blokova zarizeni, ktera nasel, a souborovy
/// system (pokud je na nekterem disku).
pub struct Devices {
    pub _devmgr: Process,
    pub _control: Channel,
    pub blocks: Vec<BlockClient>,
    pub fs: Option<FsClient>,
    pub _fsd: Option<(Process, Channel)>,
}

/// Spusti spravce zarizeni a pocka, az nahlasi vsechna zarizeni.
pub fn start_devices(startup: &Startup) -> Result<Devices> {
    let fs = Initfs::open(startup)?;
    let image = fs.find("devmgr").ok_or(Error::NotFound)?;
    let resource = startup.root_resource.as_ref().ok_or(Error::NotFound)?;
    // Spravce zarizeni dostane pristup k hardwaru, ale ne nastaveni hodin.
    let mut handles = vec![StartHandle::new(
        kind::ROOT_RESOURCE,
        0,
        resource.handle().duplicate(Some(Rights::BASIC | Rights::MANAGE))?,
    )];
    for (vmo, arg) in &startup.pci_ecam {
        handles.push(StartHandle::new(
            kind::PCI_ECAM,
            *arg,
            vmo.handle().duplicate(None)?,
        ));
    }
    let initfs = startup.initfs.as_ref().ok_or(Error::NotFound)?;
    handles.push(StartHandle::new(
        kind::INITFS,
        0,
        initfs.handle().duplicate(None)?,
    ));

    let (devmgr, control) = libos::spawn("devmgr", image, handles)?;
    let mut blocks = Vec::new();
    loop {
        let msg = control.read(Some(time::deadline_after(Duration::from_secs(10))))?;
        match &msg.data[..] {
            b"done" => break,
            b"block" => {
                for h in msg.handles {
                    match BlockClient::new(Channel::from_handle(h)) {
                        Ok(b) => blocks.push(b),
                        Err(e) => println!("init: blokove zarizeni neodpovida: {e}"),
                    }
                }
            }
            other => println!("init: neznama zprava od devmgr: {:?}", other),
        }
    }
    println!("init: {} blokovych zarizeni", blocks.len());
    Ok(Devices {
        _devmgr: devmgr,
        _control: control,
        blocks,
        fs: None,
        _fsd: None,
    })
}

/// Nastaveni casu systemu (format viz `libos::time::zone::Settings`).
///
/// TODO: cist z konfigurace systemu na disku. Do te doby pevne: QEMU ma RTC
/// v UTC; na pocitaci s Windows (RTC v mistnim case) patri `rtc = local`.
pub const TIME_SETTINGS: &str = "zone = Europe/Prague\nrtc = utc\n";

/// Bezici sluzba casu a jeji kanal.
pub struct TimeService {
    pub _process: Process,
    pub channel: Channel,
}

/// Spusti sluzbu casu: jako jedina dostane pravo nastavovat hodiny
/// (`SET_CLOCK`, bez `MANAGE`). Posle ji nastaveni a pocka na potvrzeni
/// (pripadna oprava RTC v mistnim case probehne pred startem ostatnich
/// sluzeb, takze casy souboru jsou od zacatku spravne).
pub fn start_time(startup: &Startup) -> Result<TimeService> {
    let fs = Initfs::open(startup)?;
    let image = fs.find("timed").ok_or(Error::NotFound)?;
    let resource = startup.root_resource.as_ref().ok_or(Error::NotFound)?;
    let clock = resource
        .handle()
        .duplicate(Some(Rights::BASIC | Rights::SET_CLOCK))?;
    let (process, channel) = libos::spawn("timed", image, vec![StartHandle::user(clock)])?;
    channel.write(TIME_SETTINGS.as_bytes(), Vec::new())?;
    let msg = channel.read(Some(time::deadline_after(Duration::from_secs(5))))?;
    if msg.data != b"ok" {
        println!(
            "init: sluzba casu hlasi: {}",
            core::str::from_utf8(&msg.data).unwrap_or("?")
        );
    }
    Ok(TimeService { _process: process, channel })
}

/// Podpis superbloku naseho souboroveho systemu (format v3).
const FS_MAGIC: &[u8; 8] = b"OSFS\0\0v3";

/// Ma disk nas souborovy system? Superbloky lezi v blocich 1..=8
/// (bloky po 4 KiB = sektory 8..72).
fn has_fs(b: &BlockClient) -> bool {
    let mut buf = vec![0u8; 8 * 4096];
    if b.sectors() < 72 || b.read(8, &mut buf).is_err() {
        return false;
    }
    buf.chunks(4096).any(|blk| &blk[..8] == FS_MAGIC)
}

/// Spusti sluzbu souboroveho systemu nad prvnim diskem, ktery ho obsahuje.
pub fn start_fs(startup: &Startup, devices: &mut Devices) -> Result<()> {
    let Some(i) = devices.blocks.iter().position(has_fs) else {
        println!("init: zadny disk se souborovym systemem");
        return Ok(());
    };
    let disk = devices.blocks.remove(i);
    let fs = Initfs::open(startup)?;
    let image = fs.find("fsd").ok_or(Error::NotFound)?;
    let (client, server) = Channel::create()?;
    let handles = vec![
        StartHandle::user(disk.into_channel().into_handle()),
        StartHandle::user(server.into_handle()),
    ];
    let (process, control) = libos::spawn("fsd", image, handles)?;
    // Testy potrebuji simulaci vypadku proudu.
    control.write(b"debug", Vec::new())?;
    let client = FsClient::new(client);
    let info = client.info()?;
    println!(
        "init: souborovy system '{}' pripojen (generace {})",
        info.label, info.generation
    );
    devices.fs = Some(client);
    devices._fsd = Some((process, control));
    Ok(())
}
