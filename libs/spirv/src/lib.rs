//! Vlastni minimalni assembler SPIR-V (specifikace SPIR-V 1.0, Khronos):
//! shadery pro Vulkan bez ciziho prekladace.
//!
//! Modul se sklada v poradi, ktere specifikace vyzaduje (kap. 2.4):
//! capability, rozsireni, model pameti, vstupni body, rezimy, anotace,
//! typy/konstanty/globalni promenne, funkce. [`Builder`] drzi kazdou cast
//! zvlast a `finish` je spoji s hlavickou. Typy a konstanty se nedupli-
//! kuji (stejny typ = stejne id).
//!
//! Instrukce: slovo `delka << 16 | opcode`, pak operandy (slova).
//! Retezec = UTF-8 s NUL, doplneny nulami na nasobek 4.
//!
//! Shader se pise v Rustu jako posloupnost volani (viz `blend.rs`);
//! vystup jde overit `spirv-val` (ve WSL: `spirv-val soubor.spv`).

#![cfg_attr(not(test), no_std)]

extern crate alloc;

pub mod blend;
pub mod yuv;
pub mod op;

#[cfg(test)]
mod tests;

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// Magicke cislo a verze (1.0: Vulkan 1.0+ ho prijme vzdy).
pub const MAGIC: u32 = 0x0723_0203;
pub const VERSION_1_0: u32 = 0x0001_0000;
/// Generator (0 = neregistrovany).
const GENERATOR: u32 = 0;

/// Id vysledku (`%n`).
pub type Id = u32;

/// Zakoduje retezec jako slova (NUL a doplneni).
pub fn string_words(s: &str) -> Vec<u32> {
    let mut b: Vec<u8> = s.as_bytes().into();
    b.push(0);
    while b.len() % 4 != 0 {
        b.push(0);
    }
    b.chunks(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

fn emit(out: &mut Vec<u32>, opcode: u16, operands: &[u32]) {
    let n = operands.len() + 1;
    out.push((n as u32) << 16 | opcode as u32);
    out.extend_from_slice(operands);
}

/// Klic typu nebo konstanty (deduplikace).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Type(u16, Vec<u32>),
    Const(Id, u32),
}

#[derive(Default)]
pub struct Builder {
    next: Id,
    caps: Vec<u32>,
    memory_model: Vec<u32>,
    entries: Vec<u32>,
    modes: Vec<u32>,
    annotations: Vec<u32>,
    globals: Vec<u32>,
    code: Vec<u32>,
    cache: BTreeMap<Key, Id>,
}

impl Builder {
    /// Novy modul: capability `Shader`, logicke adresovani, model GLSL450.
    pub fn new() -> Self {
        let mut b = Self { next: 1, ..Default::default() };
        b.capability(op::CAPABILITY_SHADER);
        emit(&mut b.memory_model, op::MEMORY_MODEL, &[op::ADDRESSING_LOGICAL, op::MEMORY_GLSL450]);
        b
    }

    /// Nove id.
    pub fn id(&mut self) -> Id {
        let i = self.next;
        self.next += 1;
        i
    }

    pub fn capability(&mut self, cap: u32) {
        emit(&mut self.caps, op::CAPABILITY, &[cap]);
    }

    /// Vstupni bod: model (napr. `GLCompute`), funkce, jmeno, rozhrani.
    pub fn entry_point(&mut self, model: u32, func: Id, name: &str, interface: &[Id]) {
        let mut o = alloc::vec![model, func];
        o.extend(string_words(name));
        o.extend_from_slice(interface);
        emit(&mut self.entries, op::ENTRY_POINT, &o);
    }

    pub fn execution_mode(&mut self, func: Id, mode: u32, args: &[u32]) {
        let mut o = alloc::vec![func, mode];
        o.extend_from_slice(args);
        emit(&mut self.modes, op::EXECUTION_MODE, &o);
    }

    pub fn decorate(&mut self, target: Id, decoration: u32, args: &[u32]) {
        let mut o = alloc::vec![target, decoration];
        o.extend_from_slice(args);
        emit(&mut self.annotations, op::DECORATE, &o);
    }

    pub fn member_decorate(&mut self, st: Id, member: u32, decoration: u32, args: &[u32]) {
        let mut o = alloc::vec![st, member, decoration];
        o.extend_from_slice(args);
        emit(&mut self.annotations, op::MEMBER_DECORATE, &o);
    }

    /// Typ (deduplikovany): `opcode` a operandy bez id vysledku.
    fn ty(&mut self, opcode: u16, operands: &[u32]) -> Id {
        let key = Key::Type(opcode, operands.into());
        if let Some(&i) = self.cache.get(&key) {
            return i;
        }
        let i = self.id();
        let mut o = alloc::vec![i];
        o.extend_from_slice(operands);
        emit(&mut self.globals, opcode, &o);
        self.cache.insert(key, i);
        i
    }

    pub fn type_void(&mut self) -> Id {
        self.ty(op::TYPE_VOID, &[])
    }

    pub fn type_bool(&mut self) -> Id {
        self.ty(op::TYPE_BOOL, &[])
    }

    pub fn type_uint(&mut self) -> Id {
        self.ty(op::TYPE_INT, &[32, 0])
    }

    pub fn type_vector(&mut self, component: Id, n: u32) -> Id {
        self.ty(op::TYPE_VECTOR, &[component, n])
    }

    pub fn type_pointer(&mut self, storage: u32, ty: Id) -> Id {
        self.ty(op::TYPE_POINTER, &[storage, ty])
    }

    pub fn type_function(&mut self, ret: Id, params: &[Id]) -> Id {
        let mut o = alloc::vec![ret];
        o.extend_from_slice(params);
        self.ty(op::TYPE_FUNCTION, &o)
    }

    /// Pole bez delky (posledni clen bufferu). Neduplikuje se: dekorace
    /// `ArrayStride` patri konkretnimu id.
    pub fn type_runtime_array(&mut self, element: Id) -> Id {
        let i = self.id();
        emit(&mut self.globals, op::TYPE_RUNTIME_ARRAY, &[i, element]);
        i
    }

    /// Struktura (neduplikuje se kvuli dekoracim clenu).
    pub fn type_struct(&mut self, members: &[Id]) -> Id {
        let i = self.id();
        let mut o = alloc::vec![i];
        o.extend_from_slice(members);
        emit(&mut self.globals, op::TYPE_STRUCT, &o);
        i
    }

    /// Konstanta `u32` (deduplikovana).
    pub fn const_u32(&mut self, v: u32) -> Id {
        let t = self.type_uint();
        let key = Key::Const(t, v);
        if let Some(&i) = self.cache.get(&key) {
            return i;
        }
        let i = self.id();
        emit(&mut self.globals, op::CONSTANT, &[t, i, v]);
        self.cache.insert(key, i);
        i
    }

    /// Globalni promenna (ukazatel `ptr_ty` s tridou `storage`).
    pub fn variable(&mut self, ptr_ty: Id, storage: u32) -> Id {
        let i = self.id();
        emit(&mut self.globals, op::VARIABLE, &[ptr_ty, i, storage]);
        i
    }

    /// Zacatek funkce bez parametru; vraci id funkce.
    pub fn begin_function(&mut self, ret: Id, fn_ty: Id) -> Id {
        let f = self.id();
        emit(&mut self.code, op::FUNCTION, &[ret, f, op::FUNCTION_CONTROL_NONE, fn_ty]);
        f
    }

    pub fn end_function(&mut self) {
        emit(&mut self.code, op::FUNCTION_END, &[]);
    }

    /// Navesti bloku (zadane id, at na nej jde skocit predem).
    pub fn label(&mut self, l: Id) {
        emit(&mut self.code, op::LABEL, &[l]);
    }

    /// Instrukce s vysledkem: `opcode`, typ vysledku, operandy -> nove id.
    pub fn op(&mut self, opcode: u16, result_ty: Id, operands: &[u32]) -> Id {
        let i = self.id();
        let mut o = alloc::vec![result_ty, i];
        o.extend_from_slice(operands);
        emit(&mut self.code, opcode, &o);
        i
    }

    /// Instrukce bez vysledku (store, vetveni, return).
    pub fn op_void(&mut self, opcode: u16, operands: &[u32]) {
        emit(&mut self.code, opcode, operands);
    }

    /// Hotovy modul (hlavicka + casti ve spravnem poradi).
    pub fn finish(self) -> Vec<u32> {
        let mut out = alloc::vec![MAGIC, VERSION_1_0, GENERATOR, self.next, 0];
        for part in [&self.caps, &self.memory_model, &self.entries, &self.modes, &self.annotations, &self.globals, &self.code] {
            out.extend_from_slice(part);
        }
        out
    }
}
