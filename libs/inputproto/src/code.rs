//! Kody udalosti, klaves a tlacitek (cisla z Linuxu, evdev).
//!
//! Kod klavesy oznacuje **misto na klavesnici** podle americkeho rozlozeni:
//! `KEY_Y` je klavesa vpravo od `T`, i kdyz v ceskem rozlozeni QWERTZ pise
//! `z`. Znak podle rozlozeni nese zvlast udalost `Text` a pole `ch`.

// Typy udalosti (evdev `EV_*`).
/// Konec snimku: udalosti od posledniho `SYN_REPORT` patri k sobe.
pub const EV_SYN: u16 = 0x00;
/// Klavesa nebo tlacitko: hodnota 0 uvolneni, 1 stisk, 2 opakovani.
pub const EV_KEY: u16 = 0x01;
/// Relativni osa (pohyb mysi, kolecko).
pub const EV_REL: u16 = 0x02;
/// Absolutni osa (tablet, dotyk) - zatim nepodporovano.
pub const EV_ABS: u16 = 0x03;
/// Ruzne: `MSC_SCAN` nese surovy scan kod pristi klavesy.
pub const EV_MSC: u16 = 0x04;
/// Kontrolky (Caps Lock, Num Lock).
pub const EV_LED: u16 = 0x11;
/// Automaticke opakovani (nastaveni zarizeni).
pub const EV_REP: u16 = 0x14;

/// `EV_SYN`: konec snimku.
pub const SYN_REPORT: u16 = 0;
/// `EV_SYN`: zarizeni zahodilo udalosti, stav klaves je treba overit.
pub const SYN_DROPPED: u16 = 3;
/// `EV_MSC`: surovy scan kod (napr. PS/2 sada 1, USB HID usage).
pub const MSC_SCAN: u16 = 4;

// Absolutni osy (tablet, dotyk).
pub const ABS_X: u16 = 0x00;
pub const ABS_Y: u16 = 0x01;

// Relativni osy.
pub const REL_X: u16 = 0x00;
pub const REL_Y: u16 = 0x01;
/// Vodorovne kolecko: kladne doprava.
pub const REL_HWHEEL: u16 = 0x06;
/// Svisle kolecko: kladne od uzivatele (nahoru).
pub const REL_WHEEL: u16 = 0x08;

// Tlacitka tabletu a pera.
pub const BTN_TOOL_PEN: u16 = 0x140;
/// Hrot pera nebo prst se dotyka plochy.
pub const BTN_TOUCH: u16 = 0x14A;
/// Tlacitko na tele pera.
pub const BTN_STYLUS: u16 = 0x14B;

// Tlacitka mysi.
pub const BTN_LEFT: u16 = 0x110;
pub const BTN_RIGHT: u16 = 0x111;
pub const BTN_MIDDLE: u16 = 0x112;
/// Bocni tlacitko "zpet".
pub const BTN_SIDE: u16 = 0x113;
/// Bocni tlacitko "vpred".
pub const BTN_EXTRA: u16 = 0x114;

// Klavesy (vyber, ktery system pouziva; ostatni kody se predavaji beze
// jmena).
pub const KEY_ESC: u16 = 1;
pub const KEY_1: u16 = 2;
pub const KEY_2: u16 = 3;
pub const KEY_3: u16 = 4;
pub const KEY_4: u16 = 5;
pub const KEY_5: u16 = 6;
pub const KEY_6: u16 = 7;
pub const KEY_7: u16 = 8;
pub const KEY_8: u16 = 9;
pub const KEY_9: u16 = 10;
pub const KEY_0: u16 = 11;
pub const KEY_MINUS: u16 = 12;
pub const KEY_EQUAL: u16 = 13;
pub const KEY_BACKSPACE: u16 = 14;
pub const KEY_TAB: u16 = 15;
pub const KEY_Q: u16 = 16;
pub const KEY_W: u16 = 17;
pub const KEY_E: u16 = 18;
pub const KEY_R: u16 = 19;
pub const KEY_T: u16 = 20;
pub const KEY_Y: u16 = 21;
pub const KEY_U: u16 = 22;
pub const KEY_I: u16 = 23;
pub const KEY_O: u16 = 24;
pub const KEY_P: u16 = 25;
pub const KEY_LEFTBRACE: u16 = 26;
pub const KEY_RIGHTBRACE: u16 = 27;
pub const KEY_ENTER: u16 = 28;
pub const KEY_LEFTCTRL: u16 = 29;
pub const KEY_A: u16 = 30;
pub const KEY_S: u16 = 31;
pub const KEY_D: u16 = 32;
pub const KEY_F: u16 = 33;
pub const KEY_G: u16 = 34;
pub const KEY_H: u16 = 35;
pub const KEY_J: u16 = 36;
pub const KEY_K: u16 = 37;
pub const KEY_L: u16 = 38;
pub const KEY_SEMICOLON: u16 = 39;
pub const KEY_APOSTROPHE: u16 = 40;
pub const KEY_GRAVE: u16 = 41;
pub const KEY_LEFTSHIFT: u16 = 42;
pub const KEY_BACKSLASH: u16 = 43;
pub const KEY_Z: u16 = 44;
pub const KEY_X: u16 = 45;
pub const KEY_C: u16 = 46;
pub const KEY_V: u16 = 47;
pub const KEY_B: u16 = 48;
pub const KEY_N: u16 = 49;
pub const KEY_M: u16 = 50;
pub const KEY_COMMA: u16 = 51;
pub const KEY_DOT: u16 = 52;
pub const KEY_SLASH: u16 = 53;
pub const KEY_RIGHTSHIFT: u16 = 54;
pub const KEY_KPASTERISK: u16 = 55;
pub const KEY_LEFTALT: u16 = 56;
pub const KEY_SPACE: u16 = 57;
pub const KEY_CAPSLOCK: u16 = 58;
pub const KEY_F1: u16 = 59;
pub const KEY_F2: u16 = 60;
pub const KEY_F3: u16 = 61;
pub const KEY_F4: u16 = 62;
pub const KEY_F5: u16 = 63;
pub const KEY_F6: u16 = 64;
pub const KEY_F7: u16 = 65;
pub const KEY_F8: u16 = 66;
pub const KEY_F9: u16 = 67;
pub const KEY_F10: u16 = 68;
pub const KEY_NUMLOCK: u16 = 69;
pub const KEY_SCROLLLOCK: u16 = 70;
pub const KEY_KP7: u16 = 71;
pub const KEY_KP8: u16 = 72;
pub const KEY_KP9: u16 = 73;
pub const KEY_KPMINUS: u16 = 74;
pub const KEY_KP4: u16 = 75;
pub const KEY_KP5: u16 = 76;
pub const KEY_KP6: u16 = 77;
pub const KEY_KPPLUS: u16 = 78;
pub const KEY_KP1: u16 = 79;
pub const KEY_KP2: u16 = 80;
pub const KEY_KP3: u16 = 81;
pub const KEY_KP0: u16 = 82;
pub const KEY_KPDOT: u16 = 83;
/// Klavesa navic na evropskych klavesnicich (vlevo od `Z`, 102. klavesa).
pub const KEY_102ND: u16 = 86;
pub const KEY_F11: u16 = 87;
pub const KEY_F12: u16 = 88;
pub const KEY_KPENTER: u16 = 96;
pub const KEY_RIGHTCTRL: u16 = 97;
pub const KEY_KPSLASH: u16 = 98;
pub const KEY_SYSRQ: u16 = 99;
pub const KEY_RIGHTALT: u16 = 100;
pub const KEY_HOME: u16 = 102;
pub const KEY_UP: u16 = 103;
pub const KEY_PAGEUP: u16 = 104;
pub const KEY_LEFT: u16 = 105;
pub const KEY_RIGHT: u16 = 106;
pub const KEY_END: u16 = 107;
pub const KEY_DOWN: u16 = 108;
pub const KEY_PAGEDOWN: u16 = 109;
pub const KEY_INSERT: u16 = 110;
pub const KEY_DELETE: u16 = 111;
pub const KEY_PAUSE: u16 = 119;
pub const KEY_LEFTMETA: u16 = 125;
pub const KEY_RIGHTMETA: u16 = 126;
pub const KEY_COMPOSE: u16 = 127;
// Multimedialni klavesy.
pub const KEY_MUTE: u16 = 113;
pub const KEY_VOLUMEDOWN: u16 = 114;
pub const KEY_VOLUMEUP: u16 = 115;
pub const KEY_NEXTSONG: u16 = 163;
pub const KEY_PLAYPAUSE: u16 = 164;
pub const KEY_PREVIOUSSONG: u16 = 165;
pub const KEY_STOPCD: u16 = 166;
pub const KEY_HOMEPAGE: u16 = 172;
pub const KEY_MAIL: u16 = 155;
pub const KEY_CALC: u16 = 140;
pub const KEY_SEARCH: u16 = 217;
pub const KEY_BRIGHTNESSDOWN: u16 = 224;
pub const KEY_BRIGHTNESSUP: u16 = 225;

/// Nejvyssi kod klavesy, ktery protokol prenasi (evdev `KEY_MAX`).
pub const KEY_MAX: u16 = 0x2FF;

/// Je kod tlacitko mysi (ne klavesa)?
pub const fn is_mouse_button(code: u16) -> bool {
    code >= 0x110 && code < 0x118
}

/// Je kod tlacitko (mys, joystick, gamepad, tablet, `BTN_TRIGGER_HAPPY`),
/// ne klavesa? Rozsahy evdev `BTN_MISC`..`BTN_GEAR_UP` (0x100-0x15F)
/// a `BTN_TRIGGER_HAPPY` (0x2C0-0x2E7).
pub const fn is_button(code: u16) -> bool {
    (code >= 0x100 && code < 0x160) || (code >= 0x2C0 && code < 0x2E8)
}
