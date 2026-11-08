//! Cisla instrukci a vyctu SPIR-V (specifikace SPIR-V 1.0, kap. 3), jen
//! ty, ktere shadery systemu pouzivaji.

// Instrukce (opcode).
pub const MEMORY_MODEL: u16 = 14;
pub const ENTRY_POINT: u16 = 15;
pub const EXECUTION_MODE: u16 = 16;
pub const CAPABILITY: u16 = 17;
pub const TYPE_VOID: u16 = 19;
pub const TYPE_BOOL: u16 = 20;
pub const TYPE_INT: u16 = 21;
pub const TYPE_VECTOR: u16 = 23;
pub const TYPE_RUNTIME_ARRAY: u16 = 29;
pub const TYPE_STRUCT: u16 = 30;
pub const TYPE_POINTER: u16 = 32;
pub const TYPE_FUNCTION: u16 = 33;
pub const CONSTANT: u16 = 43;
pub const FUNCTION: u16 = 54;
pub const FUNCTION_END: u16 = 56;
pub const VARIABLE: u16 = 59;
pub const LOAD: u16 = 61;
pub const STORE: u16 = 62;
pub const ACCESS_CHAIN: u16 = 65;
pub const DECORATE: u16 = 71;
pub const MEMBER_DECORATE: u16 = 72;
pub const COMPOSITE_EXTRACT: u16 = 81;
pub const I_ADD: u16 = 128;
pub const I_SUB: u16 = 130;
pub const I_MUL: u16 = 132;
pub const U_DIV: u16 = 134;
pub const LOGICAL_AND: u16 = 167;
pub const SELECT: u16 = 169;
pub const I_EQUAL: u16 = 170;
pub const U_GREATER_THAN: u16 = 172;
pub const U_GREATER_THAN_EQUAL: u16 = 174;
pub const U_LESS_THAN: u16 = 176;
pub const SHIFT_RIGHT_LOGICAL: u16 = 194;
pub const SHIFT_LEFT_LOGICAL: u16 = 196;
pub const BITWISE_OR: u16 = 197;
pub const BITWISE_AND: u16 = 199;
pub const SELECTION_MERGE: u16 = 247;
pub const LABEL: u16 = 248;
pub const BRANCH: u16 = 249;
pub const BRANCH_CONDITIONAL: u16 = 250;
pub const RETURN: u16 = 253;

// Capability, adresovani a model pameti.
pub const CAPABILITY_SHADER: u32 = 1;
pub const ADDRESSING_LOGICAL: u32 = 0;
pub const MEMORY_GLSL450: u32 = 1;

// Model vstupniho bodu a rezimy.
pub const EXECUTION_MODEL_GL_COMPUTE: u32 = 5;
pub const EXECUTION_MODE_LOCAL_SIZE: u32 = 17;

// Tridy uloziste.
pub const STORAGE_INPUT: u32 = 1;
pub const STORAGE_UNIFORM: u32 = 2;
pub const STORAGE_FUNCTION: u32 = 7;
pub const STORAGE_PUSH_CONSTANT: u32 = 9;

// Dekorace.
pub const DECORATION_BLOCK: u32 = 2;
/// Storage buffer v SPIR-V 1.0 (trida `Uniform` + `BufferBlock`).
pub const DECORATION_BUFFER_BLOCK: u32 = 3;
pub const DECORATION_ARRAY_STRIDE: u32 = 6;
pub const DECORATION_BUILT_IN: u32 = 11;
pub const DECORATION_BINDING: u32 = 33;
pub const DECORATION_DESCRIPTOR_SET: u32 = 34;
pub const DECORATION_OFFSET: u32 = 35;

pub const BUILT_IN_GLOBAL_INVOCATION_ID: u32 = 28;

pub const FUNCTION_CONTROL_NONE: u32 = 0;
pub const SELECTION_CONTROL_NONE: u32 = 0;
