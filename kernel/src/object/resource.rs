//! Resource: opravneni k primemu pristupu k hardwaru.
//!
//! Kdo ma handle na Resource s pravem MANAGE, smi vytvaret fyzicke objekty
//! pameti (registry zarizeni), souvislou pamet pro DMA s fyzickou adresou
//! a objekty preruseni. Jadro vytvori jediny korenovy Resource a da ho
//! procesu init; ten ho preda jen spravci zarizeni a ovladacum.
//!
//! Bez IOMMU muze ovladac s DMA zapsat kamkoli do pameti, takze Resource je
//! fakticky plne opravneni k systemu - davat ho jen duveryhodnym procesum.
//! TODO: jemnejsi prostredky (konkretni zarizeni, rozsah) a IOMMU.

use alloc::sync::Arc;

use super::{KernelObject, ObjectCore, ObjectType, Rights, Signals};

pub struct Resource {
    core: ObjectCore,
}

impl Resource {
    pub fn new_root() -> Arc<Self> {
        Arc::new(Self {
            core: ObjectCore::new(Signals::NONE),
        })
    }
}

impl KernelObject for Resource {
    fn core(&self) -> &ObjectCore {
        &self.core
    }

    fn object_type(&self) -> ObjectType {
        ObjectType::Resource
    }

    fn default_rights(&self) -> Rights {
        Rights::BASIC | Rights::MANAGE
    }
}
