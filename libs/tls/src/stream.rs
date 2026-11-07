//! Sifrovany proud nad `Read + Write` (rustls bez `std`).
//!
//! rustls bez `std` je stavovy automat: zaznamy prectene z TCP mu predava
//! [`TlsStream::pump`], on vraci, co je treba udelat (poslat data
//! handshaku, cist dal, predat desifrovana data). Tady je to cele zabalene
//! do blokujicich `read`/`write`.

use alloc::vec;
use alloc::vec::Vec;

use netproto::io::{Read, Write};
use rustls::client::UnbufferedClientConnection;
use rustls::unbuffered::{
    AppDataRecord, ConnectionState, EncodeError, EncryptError, InsufficientSizeError, UnbufferedStatus,
};

use crate::Error;

/// Nejvic dat v jednom zaznamu TLS (RFC 8446 5.1).
const MAX_RECORD: usize = 16 * 1024;
/// Nejvetsi zaznam na drate (data + typ + tag + hlavicka, s rezervou).
const MAX_WIRE: usize = MAX_RECORD + 256 + 5;

pub struct TlsStream<S> {
    conn: UnbufferedClientConnection,
    sock: S,
    /// Prijate, jeste nezpracovane bajty od serveru.
    incoming: Vec<u8>,
    outgoing: Vec<u8>,
    /// Desifrovana data pro `read` (od `plain_pos`).
    plain: Vec<u8>,
    plain_pos: usize,
    /// Server poslal `close_notify`.
    peer_closed: bool,
    /// Spojeni je nepouzitelne (chyba); dalsi volani ji vrati znovu.
    failed: Option<Error>,
    alpn: Option<Vec<u8>>,
}

/// Co dal udelat po jednom kroku automatu.
enum Next {
    /// Precist dalsi data ze site.
    Read,
    /// Handshake hotov, lze sifrovat (pripadne prisla data).
    Ready,
    /// Server ukoncil spojeni.
    Closed,
}

fn insufficient(e: InsufficientSizeError) -> usize {
    e.required_size
}

impl<S: Read + Write> TlsStream<S> {
    pub(crate) fn handshake(conn: UnbufferedClientConnection, sock: S) -> Result<Self, Error> {
        let mut s = Self {
            conn,
            sock,
            incoming: Vec::new(),
            outgoing: Vec::new(),
            plain: Vec::new(),
            plain_pos: 0,
            peer_closed: false,
            failed: None,
            alpn: None,
        };
        loop {
            match s.pump()? {
                Next::Ready => break,
                Next::Read => s.fill()?,
                Next::Closed => return Err(Error::Protocol(alloc::string::String::from("server ukoncil handshake"))),
            }
        }
        s.alpn = s.conn.alpn_protocol().map(|p| p.to_vec());
        Ok(s)
    }

    /// Vyjednany protokol ALPN (napr. `b"h2"`), pokud server nejaky vybral.
    pub fn alpn_protocol(&self) -> Option<&[u8]> {
        self.alpn.as_deref()
    }

    /// Spojeni navazane obnovenim drivejsi relace (PSK, bez overeni
    /// certifikatu znovu - to probehlo pri puvodnim spojeni).
    pub fn resumed(&self) -> bool {
        self.conn.handshake_kind() == Some(rustls::HandshakeKind::Resumed)
    }

    pub fn get_ref(&self) -> &S {
        &self.sock
    }

    /// Precte dalsi data ze site do `incoming`.
    fn fill(&mut self) -> Result<(), Error> {
        let start = self.incoming.len();
        self.incoming.resize(start + MAX_WIRE, 0);
        let r = self.sock.read(&mut self.incoming[start..]);
        match r {
            Ok(0) => {
                self.incoming.truncate(start);
                // Konec TCP bez close_notify: utocnik mohl odpoved zkratit.
                Err(Error::Truncated)
            }
            Ok(n) => {
                self.incoming.truncate(start + n);
                Ok(())
            }
            Err(e) => {
                self.incoming.truncate(start);
                Err(Error::Io(e))
            }
        }
    }

    fn send_outgoing(&mut self) -> Result<(), Error> {
        if !self.outgoing.is_empty() {
            self.sock.write_all(&self.outgoing)?;
            self.sock.flush()?;
            self.outgoing.clear();
        }
        Ok(())
    }

    /// Jeden pruchod automatem nad prijatymi daty.
    fn pump(&mut self) -> Result<Next, Error> {
        if let Some(e) = &self.failed {
            return Err(e.clone());
        }
        let r = self.pump_inner();
        if let Err(e) = &r {
            self.failed = Some(e.clone());
        }
        r
    }

    fn pump_inner(&mut self) -> Result<Next, Error> {
        loop {
            let UnbufferedStatus { mut discard, state } = self.conn.process_tls_records(&mut self.incoming);
            let mut transmit = false;
            let next = match state.map_err(Error::from)? {
                ConnectionState::ReadTraffic(mut r) => {
                    while let Some(rec) = r.next_record() {
                        let AppDataRecord { discard: d, payload } = rec.map_err(Error::from)?;
                        discard += d;
                        self.plain.extend_from_slice(payload);
                    }
                    None
                }
                ConnectionState::EncodeTlsData(mut e) => {
                    let start = self.outgoing.len();
                    self.outgoing.resize(start + MAX_WIRE, 0);
                    let n = match e.encode(&mut self.outgoing[start..]) {
                        Ok(n) => n,
                        // Vetsi zprava handshaku (dlouhy retez): zvetsit a znovu.
                        Err(EncodeError::InsufficientSize(s)) => {
                            self.outgoing.resize(start + insufficient(s), 0);
                            e.encode(&mut self.outgoing[start..])
                                .map_err(|e| Error::Protocol(alloc::format!("{e:?}")))?
                        }
                        Err(e) => return Err(Error::Protocol(alloc::format!("{e:?}"))),
                    };
                    self.outgoing.truncate(start + n);
                    None
                }
                ConnectionState::TransmitTlsData(t) => {
                    transmit = true;
                    t.done();
                    None
                }
                ConnectionState::BlockedHandshake => Some(Next::Read),
                ConnectionState::WriteTraffic(_) => Some(Next::Ready),
                ConnectionState::PeerClosed | ConnectionState::Closed => {
                    self.peer_closed = true;
                    Some(Next::Closed)
                }
                // Data 0-RTT jsou jen u serveru; jiny (novy) stav rustls
                // neumime - radeji chyba nez zacykleni.
                _ => return Err(Error::Protocol(alloc::string::String::from("neocekavany stav TLS"))),
            };
            self.incoming.drain(..discard);
            if transmit {
                self.send_outgoing()?;
            }
            if let Some(n) = next {
                return Ok(n);
            }
        }
    }

    /// Dovede automat do stavu "lze sifrovat": vse ostatni (data od
    /// serveru, zpravy handshaku k odeslani) zpracuje `pump`. Stavy
    /// `EncodeTlsData` se nesmi zahodit - rustls by zpravu ztratil.
    fn ready_to_write(&mut self) -> Result<(), Error> {
        let mut closed_once = false;
        loop {
            match self.pump()? {
                Next::Ready => return Ok(()),
                Next::Read => self.fill()?,
                // PeerClosed hlasi rustls jednou; potom jde dal psat
                // (polouzavreni TLS 1.3). Druhe Closed = konec obou smeru.
                Next::Closed if !closed_once => closed_once = true,
                Next::Closed => return Err(Error::Io(abi::Error::PeerClosed)),
            }
        }
    }

    /// Zasifruje a posle `data` (nejvys jeden zaznam).
    fn encrypt(&mut self, data: &[u8]) -> Result<usize, Error> {
        let data = &data[..data.len().min(MAX_RECORD)];
        self.ready_to_write()?;
        // `pump` prave skoncil ve WriteTraffic a nic noveho neprislo.
        let UnbufferedStatus { discard, state } = self.conn.process_tls_records(&mut self.incoming);
        let ConnectionState::WriteTraffic(mut w) = state.map_err(Error::from)? else {
            return Err(Error::Protocol(alloc::string::String::from("neocekavany stav TLS pred zapisem")));
        };
        let mut out = vec![0u8; data.len() + 256];
        let n = match w.encrypt(data, &mut out) {
            Ok(n) => n,
            Err(EncryptError::InsufficientSize(s)) => {
                out.resize(insufficient(s), 0);
                w.encrypt(data, &mut out).map_err(|e| Error::Protocol(alloc::format!("{e:?}")))?
            }
            Err(e) => return Err(Error::Protocol(alloc::format!("{e:?}"))),
        };
        out.truncate(n);
        self.incoming.drain(..discard);
        self.sock.write_all(&out)?;
        Ok(data.len())
    }

    /// Ukonci spojeni (`close_notify`); cteni po nem dal funguje az do
    /// `close_notify` serveru. Uz ukoncene spojeni = nic.
    pub fn close(&mut self) -> Result<(), Error> {
        match self.ready_to_write() {
            Ok(()) => {}
            Err(Error::Io(abi::Error::PeerClosed)) => return Ok(()),
            Err(e) => return Err(e),
        }
        let UnbufferedStatus { discard, state } = self.conn.process_tls_records(&mut self.incoming);
        let ConnectionState::WriteTraffic(mut w) = state.map_err(Error::from)? else {
            return Err(Error::Protocol(alloc::string::String::from("neocekavany stav TLS pred close_notify")));
        };
        let mut out = vec![0u8; 64];
        let n = w
            .queue_close_notify(&mut out)
            .map_err(|e| Error::Protocol(alloc::format!("{e:?}")))?;
        out.truncate(n);
        self.incoming.drain(..discard);
        self.sock.write_all(&out)?;
        self.sock.flush()?;
        Ok(())
    }

    /// Vrati proud pod TLS (po `close`).
    pub fn into_inner(self) -> S {
        self.sock
    }

    fn read_plain(&mut self, buf: &mut [u8]) -> usize {
        let n = buf.len().min(self.plain.len() - self.plain_pos);
        buf[..n].copy_from_slice(&self.plain[self.plain_pos..self.plain_pos + n]);
        self.plain_pos += n;
        if self.plain_pos == self.plain.len() {
            self.plain.clear();
            self.plain_pos = 0;
        }
        n
    }

    /// Precte desifrovana data (`Ok(0)` = server ukoncil spojeni
    /// `close_notify`).
    pub fn read_tls(&mut self, buf: &mut [u8]) -> Result<usize, Error> {
        if buf.is_empty() {
            return Ok(0);
        }
        loop {
            if self.plain_pos < self.plain.len() {
                return Ok(self.read_plain(buf));
            }
            if self.peer_closed {
                return Ok(0);
            }
            match self.pump()? {
                // Data mohla prijit spolu se stavem "lze sifrovat".
                Next::Ready if self.plain_pos < self.plain.len() => continue,
                Next::Ready | Next::Read => {
                    if let Err(e) = self.fill() {
                        self.failed = Some(e.clone());
                        return Err(e);
                    }
                }
                Next::Closed => continue,
            }
        }
    }
}

impl<S: Read + Write> Read for TlsStream<S> {
    fn read(&mut self, buf: &mut [u8]) -> abi::Result<usize> {
        self.read_tls(buf).map_err(|e| e.code())
    }
}

impl<S: Read + Write> Write for TlsStream<S> {
    fn write(&mut self, buf: &[u8]) -> abi::Result<usize> {
        if let Some(e) = &self.failed {
            return Err(e.code());
        }
        self.encrypt(buf).map_err(|e| {
            let code = e.code();
            self.failed = Some(e);
            code
        })
    }

    fn flush(&mut self) -> abi::Result<()> {
        self.sock.flush()
    }
}
