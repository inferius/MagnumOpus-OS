//! Kanaly (IPC).

use alloc::sync::Arc;
use alloc::vec::Vec;

use abi::syscall::{CHANNEL_MAX_DATA, CHANNEL_MAX_HANDLES};

use super::handle::arg_handle;
use super::{Args, current_process, user};
use crate::error::{Error, Result};
use crate::object::channel::{Channel, Message};
use crate::object::{Handle, KernelObject, ObjectType, Rights};
use crate::sync::SpinLock;

/// Serializuje zpravy, ktere prenaseji handly (kontrola cyklu kanalu).
static TRANSFER_LOCK: SpinLock<()> = SpinLock::new(());

/// `CHANNEL_CREATE(out_handles_ptr)`: zapise dva handly (u32).
pub fn create(a: Args) -> Result<u64> {
    let p = current_process();
    let (c0, c1) = Channel::create();
    let rights = c0.default_rights();
    let (h0, h1) = {
        let mut table = p.handles();
        let h0 = table.insert(c0, rights)?;
        match table.insert(c1, rights) {
            Ok(h1) => (h0, h1),
            Err(e) => {
                let _ = table.close(h0);
                return Err(e);
            }
        }
    };
    if let Err(e) = user::write_u32s(&p, a[0], &[h0.0, h1.0]) {
        let mut table = p.handles();
        let _ = table.close(h0);
        let _ = table.close(h1);
        return Err(e);
    }
    Ok(0)
}

/// `CHANNEL_WRITE(handle, data_ptr, data_len, handles_ptr, handles_count)`.
///
/// Predavane handly musi mit pravo TRANSFER a z tabulky odesilatele se
/// odeberou vzdy (i kdyz zapis selze, napr. PEER_CLOSED) - jinak by se
/// nedalo zarucit, ze handle nema zaroven odesilatel i prijemce.
pub fn write(a: Args) -> Result<u64> {
    let p = current_process();
    let data_len = user::check_len(a[2], CHANNEL_MAX_DATA)?;
    let count = user::check_len(a[4], CHANNEL_MAX_HANDLES)?;
    // Data zpravy lezi ve fronte v pameti jadra: ne z rezervy.
    if !crate::mm::phys::user_room(data_len.div_ceil(4096) as u64 + 1) {
        return Err(Error::NoMemory);
    }
    let data = user::read_bytes(&p, a[1], data_len)?;
    let raw_handles = user::read_u32s(&p, a[3], count)?;

    // Zpravy s handly se zapisuji jedna po druhe: kontrola cyklu nize (co
    // lezi ve fronte predavaneho kanalu) a vlozeni zpravy musi byt atomicke
    // vuci jinym prenosum kanalu.
    let _transfer = (count > 0).then(|| TRANSFER_LOCK.lock());
    let (channel, handles) = {
        let mut table = p.handles();
        let channel = table.get_as::<Channel>(arg_handle(a[0])?, Rights::WRITE)?;
        // Nejdriv vse overit, pak odebrat (vse nebo nic).
        for (i, &h) in raw_handles.iter().enumerate() {
            let object = table.get(Handle(h), Rights::TRANSFER)?;
            if channel.is_same(&object) || channel.is_peer(&object) || raw_handles[..i].contains(&h)
            {
                return Err(Error::InvalidArgs);
            }
            // Konec, v jehoz fronte uz lezi jiny konec kanalu, nejde poslat:
            // tak nikdy nevznikne cyklus (konce, ktere drzi jeden druheho ve
            // frontach a nikdy se neuvolni). Cyklus se vzdy uzavira poslanim
            // konce, ktery uz v retezci neco drzi.
            if object.object_type() == ObjectType::Channel {
                let any: Arc<dyn core::any::Any + Send + Sync> = object.clone();
                if any.downcast::<Channel>().is_ok_and(|c| c.holds_channels()) {
                    return Err(Error::BadState);
                }
            }
        }
        let handles: Vec<(Arc<dyn KernelObject>, Rights)> = raw_handles
            .iter()
            .map(|&h| table.take(Handle(h)).expect("overeno vyse"))
            .collect();
        (channel, handles)
    };
    channel.write(Message { data, handles })?;
    Ok(0)
}

/// `CHANNEL_READ(handle, data_ptr, data_cap, handles_ptr, handles_cap,
/// out_actual_ptr)`: do `out_actual_ptr` zapise dva u32 - skutecnou delku
/// dat a pocet handlu (i pri BUFFER_TOO_SMALL, aby si proces zvetsil buffer).
pub fn read(a: Args) -> Result<u64> {
    let p = current_process();
    let channel = p
        .handles()
        .get_as::<Channel>(arg_handle(a[0])?, Rights::READ)?;
    let (data_cap, handles_cap) = (a[2] as usize, a[4] as usize);
    let msg = match channel.read(data_cap, handles_cap) {
        Ok(m) => m,
        Err(Error::BufferTooSmall) => {
            if let Some((d, h)) = channel.peek_size() {
                user::write_u32s(&p, a[5], &[d as u32, h as u32])?;
            }
            return Err(Error::BufferTooSmall);
        }
        Err(e) => return Err(e),
    };
    // Data se kopiruji pred vlozenim handlu: pri chybe kopirovani se zprava
    // zahodi i s handly (objekty se uvolni), nic nezustane napul.
    user::write_bytes(&p, a[1], &msg.data)?;
    let mut values = Vec::with_capacity(msg.handles.len());
    {
        let mut table = p.handles();
        for (object, rights) in msg.handles {
            values.push(table.insert(object, rights)?.0);
        }
    }
    user::write_u32s(&p, a[3], &values)?;
    user::write_u32s(&p, a[5], &[msg.data.len() as u32, values.len() as u32])?;
    Ok(0)
}
