//! Model zarizeni pro simulaci: co zarizeni dela pri pristupu do registru
//! a jak pracuje se sdilenou pameti a prerusenimi.

use alloc::vec::Vec;

use crate::DriverError;

use super::State;

/// Model zarizeni. Implementuje ho autor ovladace (nebo vlakno, ktere pise
/// ovladac pro hardware, ktery QEMU neemuluje) podle datasheetu.
///
/// Velikosti pristupu jsou 1, 2, 4 nebo 8 bajtu; hodnota je v dolnich
/// bitech `u64`.
pub trait DeviceModel: 'static {
    /// Velikost pametoveho BARu `index`, `None` = BAR neni.
    fn bar_size(&self, index: u8) -> Option<u64>;

    /// Cteni registru.
    fn read(&mut self, bus: &mut Bus, bar: u8, off: u64, size: u8) -> u64;

    /// Zapis registru.
    fn write(&mut self, bus: &mut Bus, bar: u8, off: u64, size: u8, value: u64);

    /// Ubehl cas (`bus.now()` je novy cas). Model tu dokonci rozpracovanou
    /// praci (vypocet, prenos DMA, prijem paketu) a vyvola preruseni.
    fn tick(&mut self, _bus: &mut Bus) {}

    /// Model pro `downcast` v testech ([`super::Sim::with_model`]).
    fn as_any(&mut self) -> &mut dyn core::any::Any;
}

/// Pristup zarizeni k okoli: DMA, preruseni, cas. Kazdy pristup DMA se
/// kontroluje proti pameti, kterou ovladac alokoval (jako IOMMU); pristup
/// jinam se zaznamena jako [`DriverError::DmaFault`] a vrati chybu.
pub struct Bus<'a> {
    pub(super) state: &'a mut State,
}

impl Bus<'_> {
    /// Virtualni cas v ns.
    pub fn now(&self) -> u64 {
        self.state.now
    }

    /// Zarizeni cte `buf.len()` bajtu z adresy `iova`.
    pub fn dma_read(&mut self, iova: u64, buf: &mut [u8]) -> Result<(), DriverError> {
        match self.state.dma.find(iova, buf.len() as u64) {
            Some((i, off)) => {
                let r = &self.state.dma.regions[i];
                buf.copy_from_slice(&r.data[off..off + buf.len()]);
                Ok(())
            }
            None => Err(self.state.fault(iova, buf.len() as u64, false)),
        }
    }

    /// Zarizeni zapisuje `data` na adresu `iova`.
    pub fn dma_write(&mut self, iova: u64, data: &[u8]) -> Result<(), DriverError> {
        match self.state.dma.find(iova, data.len() as u64) {
            Some((i, off)) => {
                let r = &mut self.state.dma.regions[i];
                r.data[off..off + data.len()].copy_from_slice(data);
                Ok(())
            }
            None => Err(self.state.fault(iova, data.len() as u64, true)),
        }
    }

    /// Cislo little endian z DMA pameti.
    pub fn dma_read_u64(&mut self, iova: u64, size: u8) -> Result<u64, DriverError> {
        let mut b = [0u8; 8];
        self.dma_read(iova, &mut b[..size as usize])?;
        Ok(u64::from_le_bytes(b))
    }

    pub fn dma_write_u64(&mut self, iova: u64, size: u8, v: u64) -> Result<(), DriverError> {
        self.dma_write(iova, &v.to_le_bytes()[..size as usize])
    }

    /// Vyvola preruseni (MSI-X polozka / vektor `vector`). Preruseni, ktere
    /// ovladac nezapnul, se zahodi a zaloguje (na skutecnem stroji by
    /// zmizelo stejne).
    pub fn raise_irq(&mut self, vector: u16) {
        if vector < self.state.irqs_enabled {
            if !self.state.pending.contains(&vector) {
                self.state.pending.push_back(vector);
            }
        } else {
            self.state.logs.push(alloc::format!(
                "sim: zarizeni vyvolalo preruseni {vector}, ovladac ho nezapnul (zahozeno)"
            ));
        }
    }

    /// Poznamka do logu simulace (ladeni modelu).
    pub fn note(&mut self, msg: &str) {
        self.state.logs.push(alloc::format!("sim: {msg}"));
    }
}

/// Jedna DMA oblast (alokace ovladace).
pub(crate) struct Region {
    pub iova: u64,
    pub data: Vec<u8>,
    pub name: &'static str,
    /// Buffer zanikl (odmapovano): zarizeni ho uz nevidi.
    pub gone: bool,
}

/// Vsechna DMA pamet ovladace.
#[derive(Default)]
pub(crate) struct DmaSpace {
    pub regions: Vec<Region>,
    pub next_iova: u64,
    pub used: u64,
}

impl DmaSpace {
    /// Oblast a offset pro pristup `iova..iova+len`; cely pristup musi lezet
    /// v jedne oblasti.
    pub fn find(&self, iova: u64, len: u64) -> Option<(usize, usize)> {
        self.regions.iter().position(|r| {
            !r.gone && iova >= r.iova && crate::io::fits(iova - r.iova, len, r.data.len() as u64)
        })
        .map(|i| (i, (iova - self.regions[i].iova) as usize))
    }
}
