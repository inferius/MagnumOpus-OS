//! Skladani mrtvych klaves: diakriticke znamenko + pismeno = znak.

/// Carka (acute), hacek (caron), prehlaska (diaeresis): pary (pismeno,
/// vysledek). Tabulky obsahuji cestinu, slovenstinu a bezne evropske
/// znaky se stejnym znamenkem.
const ACUTE: &[(char, char)] = &[
    ('a', 'á'),
    ('e', 'é'),
    ('i', 'í'),
    ('o', 'ó'),
    ('u', 'ú'),
    ('y', 'ý'),
    ('l', 'ĺ'),
    ('r', 'ŕ'),
    ('c', 'ć'),
    ('n', 'ń'),
    ('s', 'ś'),
    ('z', 'ź'),
    ('A', 'Á'),
    ('E', 'É'),
    ('I', 'Í'),
    ('O', 'Ó'),
    ('U', 'Ú'),
    ('Y', 'Ý'),
    ('L', 'Ĺ'),
    ('R', 'Ŕ'),
    ('C', 'Ć'),
    ('N', 'Ń'),
    ('S', 'Ś'),
    ('Z', 'Ź'),
];

const CARON: &[(char, char)] = &[
    ('c', 'č'),
    ('d', 'ď'),
    ('e', 'ě'),
    ('l', 'ľ'),
    ('n', 'ň'),
    ('r', 'ř'),
    ('s', 'š'),
    ('t', 'ť'),
    ('z', 'ž'),
    ('C', 'Č'),
    ('D', 'Ď'),
    ('E', 'Ě'),
    ('L', 'Ľ'),
    ('N', 'Ň'),
    ('R', 'Ř'),
    ('S', 'Š'),
    ('T', 'Ť'),
    ('Z', 'Ž'),
];

const DIAERESIS: &[(char, char)] = &[
    ('a', 'ä'),
    ('e', 'ë'),
    ('i', 'ï'),
    ('o', 'ö'),
    ('u', 'ü'),
    ('y', 'ÿ'),
    ('A', 'Ä'),
    ('E', 'Ë'),
    ('I', 'Ï'),
    ('O', 'Ö'),
    ('U', 'Ü'),
];

/// Slozi znamenko `accent` (samostatny znak, napr. `´`) se znakem `ch`.
/// `None` = kombinace neexistuje.
pub fn compose(accent: char, ch: char) -> Option<char> {
    let table = match accent {
        '´' => ACUTE,
        'ˇ' => CARON,
        '¨' => DIAERESIS,
        _ => return None,
    };
    table.iter().find(|(c, _)| *c == ch).map(|&(_, r)| r)
}
