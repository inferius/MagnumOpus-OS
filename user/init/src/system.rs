//! Start systemovych sluzeb: sluzba casu, spravce zarizeni, souborovy
//! system a sluzba site.

use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use blockproto::BlockClient;
use fsproto::FsClient;
use netproto::Net;
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
    /// Kanaly k ovladacum sitovych karet (dokud je neprevezme sluzba site).
    pub nets: Vec<Channel>,
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
    let mut nets = Vec::new();
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
            b"net" => nets.extend(msg.handles.into_iter().map(Channel::from_handle)),
            other => println!("init: neznama zprava od devmgr: {:?}", other),
        }
    }
    println!(
        "init: {} blokovych zarizeni, {} sitovych karet",
        blocks.len(),
        nets.len()
    );
    Ok(Devices {
        _devmgr: devmgr,
        _control: control,
        blocks,
        nets,
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

/// Bezici sluzba site a korenova relace (vsechna prava). Programy dostavaji
/// jen uzsi relace (`Net::restrict`).
pub struct NetService {
    pub _process: Process,
    pub _control: Channel,
    pub root: Net,
}

/// Spusti sluzbu site nad prvni sitovou kartou.
///
/// TODO: vic karet (smerovani mezi rozhranimi), nastaveni site z konfigurace
/// systemu (staticka adresa, servery DNS a NTP).
pub fn start_net(startup: &Startup, devices: &mut Devices) -> Result<NetService> {
    if devices.nets.is_empty() {
        return Err(Error::NotFound);
    }
    let link = devices.nets.remove(0);
    let fs = Initfs::open(startup)?;
    let image = fs.find("netd").ok_or(Error::NotFound)?;
    let (root, server) = Channel::create()?;
    let handles = vec![
        StartHandle::user(link.into_handle()),
        StartHandle::user(server.into_handle()),
    ];
    let (process, control) = libos::spawn("netd", image, handles)?;
    let msg = control.read(Some(time::deadline_after(Duration::from_secs(10))))?;
    if msg.data != b"ok" {
        println!(
            "init: sluzba site hlasi: {}",
            core::str::from_utf8(&msg.data).unwrap_or("?")
        );
        return Err(Error::IoError);
    }
    Ok(NetService {
        _process: process,
        _control: control,
        root: Net::new(root),
    })
}

/// Seridi hodiny podle serveru NTP: dotaz pres sit, nastaveni pres sluzbu
/// casu (jen ta smi `CLOCK_SET`). Vraci posun hodin pred nastavenim v ns.
pub fn sync_time_ntp(
    net: &Net,
    time_service: &TimeService,
    server: core::net::SocketAddr,
) -> Result<i128> {
    let sync = net.ntp_query(server, Duration::from_secs(5))?;
    let mut msg = Vec::with_capacity(16);
    msg.extend_from_slice(b"set");
    msg.extend_from_slice(&sync.time.secs.to_le_bytes());
    msg.extend_from_slice(&sync.time.nanos.to_le_bytes());
    // Do RTC nezapisovat: cas ze site plati jen do restartu, RTC opravi
    // az nastaveni systemu (TODO: zapis do RTC podle konfigurace).
    msg.push(0);
    time_service.channel.write(&msg, Vec::new())?;
    let reply = time_service
        .channel
        .read(Some(time::deadline_after(Duration::from_secs(5))))?;
    if !reply.data.starts_with(b"ok") {
        println!(
            "init: sluzba casu odmitla cas z NTP: {}",
            core::str::from_utf8(&reply.data).unwrap_or("?")
        );
        return Err(Error::IoError);
    }
    println!(
        "init: hodiny serizeny podle NTP {} (posun {} ms, zpozdeni {} ms)",
        server,
        sync.offset_ns / 1_000_000,
        sync.delay_ns / 1_000_000
    );
    Ok(sync.offset_ns)
}
