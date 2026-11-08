//! Doprava ke Claude API: TLS (`libs/tls`) + HTTP (`libs/httpc`) pres
//! relaci site z manifestu (jen `api.anthropic.com:443`).
//!
//! Vsechna volani poskytovatele jdou pres jeden zamek ([`NetCtx`]) -
//! verze 1 posila pozadavky relaci po jednom (jednoduche, pametove
//! uspornejsi; soubezne relace na sebe pockaji).

use alloc::format;
use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::time::Duration;

use agentproto::claude::Transport;
use agentproto::provider::ProviderError;
use httpc::{Client, Method, Request, Url};
use netproto::Net;
use tls::revocation::RevocationSource;
use tls::roots::Roots;
use web::Web;

pub struct NetCtx {
    pub net: Net,
    pub roots: Roots,
    pub revocation: Option<Arc<dyn RevocationSource>>,
}

// SAFETY: NetCtx je vzdy za zamkem (`spin::Mutex`) a pouziva ho jen vlakno,
// ktere zamek drzi; kanaly a klice uvnitr nejsou vazane na vlakno.
unsafe impl Send for NetCtx {}

/// Doprava nad sdilenym kontextem site.
pub struct Https {
    pub ctx: Option<Arc<spin::Mutex<NetCtx>>>,
}

impl Transport for Https {
    fn post(&mut self, url: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<(u16, Vec<u8>), ProviderError> {
        let ctx = self.ctx.as_ref().ok_or_else(|| ProviderError::Transport(String::from("agentd nema sit")))?;
        let ctx = ctx.lock();
        let tls = tls::Client::with_revocation(&tls::LIBOS, &ctx.roots, httpc::ALPN, ctx.revocation.clone())
            .map_err(|e| ProviderError::Transport(format!("TLS: {e}")))?;
        let mut web = Web::new(&ctx.net, &tls);
        // Odpoved modelu muze trvat minuty (bez streamovani).
        web.timeout = Duration::from_secs(600);
        let limits = httpc::Limits { max_body: agentproto::claude::MAX_RESPONSE, ..httpc::Limits::default() };
        let mut client = Client::new(web, limits);
        let url = Url::parse(url).map_err(|e| ProviderError::Request(format!("{e}")))?;
        let mut req = Request { body: body.to_vec(), ..Request::new(Method::Post, url) };
        for (k, v) in headers {
            req = req.header(k, v);
        }
        let resp = client.request(req).map_err(|e| ProviderError::Transport(format!("{e}")))?;
        Ok((resp.status, resp.body))
    }
}
