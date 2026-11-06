//! Start systemovych sluzeb: spravce zarizeni a (pozdeji) dalsich.

use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;

use blockproto::BlockClient;
use libos::abi::bootstrap::kind;
use libos::initfs::Initfs;
use libos::spawn::StartHandle;
use libos::{Channel, Error, Process, Result, Startup, println, time};

/// Bezici spravce zarizeni a blokova zarizeni, ktera nasel.
pub struct Devices {
    pub _devmgr: Process,
    pub _control: Channel,
    pub blocks: Vec<BlockClient>,
}

/// Spusti spravce zarizeni a pocka, az nahlasi vsechna zarizeni.
pub fn start_devices(startup: &Startup) -> Result<Devices> {
    let fs = Initfs::open(startup)?;
    let image = fs.find("devmgr").ok_or(Error::NotFound)?;
    let resource = startup.root_resource.as_ref().ok_or(Error::NotFound)?;
    let mut handles = vec![StartHandle::new(
        kind::ROOT_RESOURCE,
        0,
        resource.handle().duplicate(None)?,
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
    })
}
