//! Render uzel: klienti s kontextem GPU (protokol `renderproto`), u Venusu
//! Vulkan pres virtio-gpu.
//!
//! Jeden kanal = jeden klient = nejvys jeden kontext. Buffer s `MAPPABLE`
//! ovladac namapuje do oblasti sdilene pameti zarizeni (BAR, id
//! `SHM_ID_HOST_VISIBLE`) a klientovi posle fyzickou VMO tohoto okna.
//! Zavreni kanalu znici buffery i kontext.
//!
//! VMO okna dostane typ cache podle `map_info` zarizeni. Pri `DESTROY`
//! a konci klienta ji ovladac odvola (`VMO_REVOKE`): mapovani zmizi i
//! v procesu klienta, takze rozsah BARu muze bezpecne dostat jiny klient.
//!
//! `SUBMIT` s ploty jadra ([`Job`]):
//! - ovladac prikaz prijme (odpoved `SUBMITTED` hned) a ceka na vstupni
//!   ploty: registrace `FENCE_WAIT_ASYNC` do portu ovladace, hlavni smycka
//!   po kazdem paketu prohleda cekajici prikazy ([`Client::ready_jobs`]),
//! - po splneni vsech posle prikaz zarizeni s plotem virtio na ose
//!   `ring_idx` = fronta; po jeho splneni posune vystupni ploty,
//! - chyba nebo opusteni vstupniho plotu, chyba zarizeni nebo prekroceni
//!   `renderproto::HANG_TIMEOUT` (cekani i beh) = vystupni ploty skonci
//!   `FENCE_FAILED` a ovladac zaloguje duvod. Nikdo tak neceka do
//!   nekonecna, ani kdyz klient poslal cyklus plotu.
//! - Klient smi mit nejvys [`MAX_JOBS`] nedokoncenych prikazu (pak
//!   `SHOULD_WAIT`).

use alloc::vec::Vec;

use libos::fence::Fence;
use libos::{Channel, Error, Handle, Port, Result, Vmo, println};
use renderproto::{Reply, Request, Uapi, flags, mem};
use vgpu::blob;

use crate::device::Gpu;

/// Nejvetsi buffer (zarizeni nema vic sdilene pameti nez par stovek MiB).
const MAX_BUFFER: u64 = 256 << 20;
/// Nejvic nedokoncenych `SUBMIT` jednoho klienta.
pub const MAX_JOBS: usize = 16;
/// Nejvic front (os `ring_idx` virtio-gpu je 0..63).
const MAX_QUEUE: u32 = 63;

struct Buffer {
    resource: u32,
    /// Offset a velikost ve sdilene pameti (namapovany buffer).
    shm: Option<(u64, u64)>,
    /// Vlastni handle VMO okna (pro odvolani).
    vmo: Option<Vmo>,
}

/// Prijaty `SUBMIT`.
struct Job {
    seq: u32,
    queue: u8,
    data: Vec<u8>,
    wait: Vec<(Fence, u64)>,
    signal: Vec<(Fence, u64)>,
    /// Kdy ho prohlasit za zaseknuty (ns od startu).
    deadline: u64,
    /// Odeslany zarizeni (ceka na plot virtio).
    sent: bool,
}

pub struct Client {
    pub ch: Channel,
    /// Jednoznacne id klienta (horni pulka znacky asynchronnich prikazu).
    pub id: u64,
    ctx: Option<u32>,
    buffers: Vec<Buffer>,
    jobs: Vec<Job>,
    next_seq: u32,
}

/// Rozhodnuti o cekacich plotech prikazu.
enum Waits {
    Pending,
    Ready,
    /// Nektery plot skoncil chybou nebo byl opusten.
    Failed,
}

impl Client {
    pub fn new(ch: Channel, id: u64) -> Self {
        Self { ch, id, ctx: None, buffers: Vec::new(), jobs: Vec::new(), next_seq: 1 }
    }

    /// Obslouzi zpravu; odpoved a handly k ni.
    pub fn handle(&mut self, gpu: &mut Gpu, port: &Port, data: &[u8], handles: Vec<Handle>) -> (Reply, Vec<Handle>) {
        let op = renderproto::op_of(data);
        match Request::decode(data).and_then(|req| self.request(gpu, port, req, handles)) {
            Ok(x) => x,
            Err(code) => (Reply::Error { op, code }, Vec::new()),
        }
    }

    fn request(&mut self, gpu: &mut Gpu, port: &Port, req: Request, handles: Vec<Handle>) -> Result<(Reply, Vec<Handle>)> {
        let none = |r: Reply| Ok((r, Vec::new()));
        match req {
            Request::Info => none(Reply::Info {
                class: renderproto::class::VIRTIO_GPU,
                uapis: gpu
                    .capsets
                    .iter()
                    .take(renderproto::MAX_UAPIS)
                    .map(|&(id, version, size)| Uapi { id, version, size })
                    .collect(),
                shm_size: gpu.shm.map_or(0, |s| s.len),
            }),
            Request::Query { key, a: id, b: version } => {
                if key != renderproto::query::CAPSET {
                    return Err(Error::NotSupported);
                }
                let &(_, max, size) = gpu.capsets.iter().find(|c| c.0 == id).ok_or(Error::NotFound)?;
                if version > max || size as usize > renderproto::MAX_QUERY {
                    return Err(Error::InvalidArgs);
                }
                none(Reply::Query(gpu.get_capset(id, version, size)?))
            }
            Request::CreateContext { uapi } => {
                if self.ctx.is_some() {
                    return Err(Error::AlreadyExists);
                }
                if !gpu.context_init || !gpu.capsets.iter().any(|c| c.0 == uapi) {
                    return Err(Error::NotSupported);
                }
                // Venus bez blobu a sdilene pameti nema kam psat odpovedi.
                if uapi == vgpu::CAPSET_VENUS && (!gpu.blob || gpu.shm.is_none()) {
                    return Err(Error::NotSupported);
                }
                self.ctx = Some(gpu.create_context(uapi)?);
                none(Reply::Context)
            }
            Request::CreateBuffer { mem: m, flags: f, driver_id, size } => {
                let ctx = self.ctx.ok_or(Error::BadState)?;
                if m != mem::HOST3D || !gpu.blob {
                    return Err(Error::NotSupported);
                }
                if size == 0 || size % 4096 != 0 || size > MAX_BUFFER {
                    return Err(Error::InvalidArgs);
                }
                self.create_buffer(gpu, ctx, f, driver_id, size)
            }
            // Import pameti klienta (blob GUEST nad DMA_MAP) prijde s 6a-4
            // a NVK (`docs/cs/render.md`).
            Request::ImportBuffer { .. } => Err(Error::NotSupported),
            Request::Destroy { buffer } => {
                let ctx = self.ctx.ok_or(Error::BadState)?;
                let i = self.buffers.iter().position(|b| b.resource == buffer).ok_or(Error::NotFound)?;
                let b = self.buffers.remove(i);
                destroy_buffer(gpu, ctx, &b);
                none(Reply::Done { op: renderproto::op::DESTROY })
            }
            // virtio-gpu nema VM GPU: adresy ridi renderer hostitele.
            Request::Bind { .. } => {
                self.ctx.ok_or(Error::BadState)?;
                none(Reply::Done { op: renderproto::op::BIND })
            }
            Request::Submit { queue, wait, signal, data } => {
                let ctx = self.ctx.ok_or(Error::BadState)?;
                self.submit(gpu, port, ctx, queue, wait, signal, data, handles)?;
                none(Reply::Submitted)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn submit(
        &mut self,
        gpu: &mut Gpu,
        port: &Port,
        ctx: u32,
        queue: u32,
        wait: Vec<u64>,
        signal: Vec<u64>,
        data: Vec<u8>,
        handles: Vec<Handle>,
    ) -> Result<()> {
        if queue > MAX_QUEUE || handles.len() != wait.len() + signal.len() {
            return Err(Error::InvalidArgs);
        }
        if self.jobs.len() >= MAX_JOBS {
            return Err(Error::ShouldWait);
        }
        // Handly zpravy: cekaci ploty, pak signalizacni (prava overi jadro
        // pri FENCE_WAIT_ASYNC / FENCE_SIGNAL).
        let mut fences = handles.into_iter().map(Fence::from_handle);
        let wait: Vec<(Fence, u64)> = fences.by_ref().zip(wait).collect();
        let signal: Vec<(Fence, u64)> = fences.zip(signal).collect();
        if data.is_empty() && signal.is_empty() && wait.is_empty() {
            return Ok(());
        }
        let seq = self.next_seq;
        self.next_seq = self.next_seq.wrapping_add(1).max(1);
        // Klic paketu: jen probuzeni smycky, stav se zjisti dotazem.
        for (f, v) in &wait {
            f.wait_async(*v, port, self.id)?;
        }
        let deadline = libos::time::deadline_after(renderproto::HANG_TIMEOUT);
        self.jobs.push(Job { seq, queue: queue as u8, data, wait, signal, deadline, sent: false });
        self.ready_jobs(gpu, ctx);
        Ok(())
    }

    /// Prikazy, jejichz cekaci ploty jsou splnene, posle zarizeni; prikazy
    /// s chybnym plotem ukonci chybou. Poradi v ramci fronty zustava: prikaz
    /// nepredbehne drivejsi prikaz stejne fronty, ktery jeste ceka.
    pub fn ready_jobs(&mut self, gpu: &mut Gpu, ctx: u32) {
        let mut blocked = 0u64;
        let mut i = 0;
        while i < self.jobs.len() {
            let j = &self.jobs[i];
            let bit = 1u64 << j.queue;
            if j.sent || blocked & bit != 0 {
                if !j.sent {
                    // Zablokovany prikaz blokuje i dalsi prikazy fronty.
                    blocked |= bit;
                }
                i += 1;
                continue;
            }
            match waits(&j.wait) {
                Waits::Pending => {
                    blocked |= bit;
                    i += 1;
                }
                Waits::Failed => {
                    let j = self.jobs.remove(i);
                    println!("virtio-gpu: prikaz klienta {} neodeslan: vstupni plot skoncil chybou", self.id);
                    fail(&j.signal);
                }
                Waits::Ready => {
                    let tag = self.id << 32 | j.seq as u64;
                    // S vystupnimi ploty vzdy pres plot zarizeni, i s prazdnymi
                    // daty: plot fronty `ring_idx` se splni az po vsi praci
                    // fronty odeslane predtim (Venus: VkQueue svazana s osou).
                    let r = if j.signal.is_empty() {
                        gpu.submit(ctx, &j.data).map(|()| false)
                    } else {
                        gpu.submit_fenced(ctx, j.queue, &j.data, tag).map(|()| true)
                    };
                    match r {
                        Ok(true) => {
                            self.jobs[i].sent = true;
                            i += 1;
                        }
                        Ok(false) => {
                            let j = self.jobs.remove(i);
                            signal(&j.signal);
                        }
                        Err(e) => {
                            let j = self.jobs.remove(i);
                            println!("virtio-gpu: prikaz klienta {} odmitnut zarizenim: {e}", self.id);
                            fail(&j.signal);
                        }
                    }
                }
            }
        }
    }

    /// Kontext klienta (pro `ready_jobs` z hlavni smycky).
    pub fn context(&self) -> Option<u32> {
        self.ctx
    }

    /// Dokonceny prikaz se znackou `seq`: vystupni ploty posunout, nebo
    /// ukoncit chybou podle odpovedi zarizeni.
    pub fn complete(&mut self, seq: u32, resp_type: u32) {
        let Some(i) = self.jobs.iter().position(|j| j.sent && j.seq == seq) else {
            // Uz ukonceny po HANG_TIMEOUT: pozde dokonceny prikaz nic nemeni.
            return;
        };
        let j = self.jobs.remove(i);
        match crate::device::completion_result(resp_type) {
            Ok(()) => signal(&j.signal),
            Err(e) => {
                println!("virtio-gpu: prikaz klienta {} skoncil chybou zarizeni: {e}", self.id);
                fail(&j.signal);
            }
        }
    }

    /// Nejblizsi termin zaseknuti.
    pub fn next_deadline(&self) -> Option<u64> {
        self.jobs.iter().map(|j| j.deadline).min()
    }

    /// Prikazy po terminu: vystupni ploty `FENCE_FAILED` (duvod hang).
    pub fn expire(&mut self, now: u64) {
        let mut i = 0;
        while i < self.jobs.len() {
            if self.jobs[i].deadline > now {
                i += 1;
                continue;
            }
            let j = self.jobs.remove(i);
            println!(
                "virtio-gpu: prikaz klienta {} fronta {}: hang ({} s {}), vystupni ploty selhaly",
                self.id,
                j.queue,
                renderproto::HANG_TIMEOUT.as_secs(),
                if j.sent { "na zarizeni" } else { "cekani na vstupni ploty" }
            );
            fail(&j.signal);
        }
    }

    fn create_buffer(&mut self, gpu: &mut Gpu, ctx: u32, f: u32, driver_id: u64, size: u64) -> Result<(Reply, Vec<Handle>)> {
        let mappable = f & flags::MAPPABLE != 0;
        let shm = gpu.shm.ok_or(Error::NotSupported);
        let offset = if mappable {
            shm?;
            Some(gpu.shm_alloc.as_mut().and_then(|a| a.alloc(size)).ok_or(Error::NoMemory)?)
        } else {
            None
        };
        let resource = gpu.new_resource();
        let info = blob::BlobInfo { resource, mem: blob::BLOB_MEM_HOST3D, flags: f, blob_id: driver_id, size };
        if let Err(e) = gpu.simple_cmd(&blob::resource_create_blob(ctx, &info, &[])) {
            if let (Some(o), Some(a)) = (offset, gpu.shm_alloc.as_mut()) {
                a.release(o, size);
            }
            return Err(e);
        }
        let mut b = Buffer { resource, shm: None, vmo: None };
        let mut handles = Vec::new();
        let mut map_info = 0;
        if let Some(off) = offset {
            let mapped = gpu.map_blob(ctx, resource, off).and_then(|mi| {
                map_info = mi;
                b.shm = Some((off, size));
                gpu.shm_vmo(off, size, mi)
            });
            match mapped.and_then(|vmo| vmo.handle().duplicate(None).map(|h| (vmo, h))) {
                Ok((vmo, h)) => {
                    handles.push(h);
                    b.vmo = Some(vmo);
                }
                Err(e) => {
                    if b.shm.is_none()
                        && let Some(a) = gpu.shm_alloc.as_mut()
                    {
                        a.release(off, size);
                    }
                    destroy_buffer(gpu, ctx, &b);
                    return Err(e);
                }
            }
        }
        self.buffers.push(b);
        Ok((Reply::Buffer { buffer: resource, size, map_info }, handles))
    }

    /// Konec klienta: nedokoncene prikazy chybou, buffery a kontext pryc.
    pub fn close(&mut self, gpu: &mut Gpu) {
        for j in core::mem::take(&mut self.jobs) {
            fail(&j.signal);
        }
        let Some(ctx) = self.ctx.take() else { return };
        for b in core::mem::take(&mut self.buffers) {
            destroy_buffer(gpu, ctx, &b);
        }
        gpu.destroy_context(ctx);
    }
}

/// Stav cekacich plotu (dotaz `FENCE_QUERY`; chybny handle = chyba).
fn waits(list: &[(Fence, u64)]) -> Waits {
    let mut ready = true;
    for (f, v) in list {
        match f.query() {
            Ok(i) if i.value >= *v => {}
            Ok(i) if i.status != 0 => return Waits::Failed,
            Ok(_) => ready = false,
            Err(_) => return Waits::Failed,
        }
    }
    if ready { Waits::Ready } else { Waits::Pending }
}

fn signal(list: &[(Fence, u64)]) {
    for (f, v) in list {
        // Plot klienta muze byt uz dal nebo bez prava SIGNAL: jeho vec.
        let _ = f.signal(*v);
    }
}

fn fail(list: &[(Fence, u64)]) {
    for (f, v) in list {
        let _ = f.fail(*v);
    }
}

fn destroy_buffer(gpu: &mut Gpu, ctx: u32, b: &Buffer) {
    // Nejdriv klient ztrati pristup, pak se blob odmapuje v zarizeni a rozsah
    // se vrati (muze ho dostat jiny klient).
    // Kdyz odvolani selze, rozsah se uz nikomu neda (karantena).
    let revoked = b.vmo.as_ref().is_none_or(|v| gpu.revoke(v).is_ok());
    if let Some((off, size)) = b.shm {
        let _ = gpu.simple_cmd(&blob::resource_unmap_blob(ctx, b.resource));
        if revoked && let Some(a) = gpu.shm_alloc.as_mut() {
            a.release(off, size);
        }
    }
    let _ = gpu.simple_cmd(&vgpu::resource_unref(b.resource));
}
