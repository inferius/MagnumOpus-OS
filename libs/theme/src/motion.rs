//! Pohyb: delky animaci a krivky (cubic-bezier) z tokenu.

use conf::Table;

use crate::{Error, Result, f, u};

/// Krivka `cubic-bezier(x1, y1, x2, y2)` (body 0,0 a 1,1 pevne).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Curve(pub [f32; 4]);

impl Curve {
    fn parse(t: &Table, path: &str) -> Result<Self> {
        let s = crate::s(t, path)?;
        let mut v = [0f32; 4];
        let mut it = s.split_whitespace();
        for x in v.iter_mut() {
            *x = it.next().and_then(|n| n.parse().ok()).ok_or_else(|| Error::Invalid(path.into()))?;
        }
        if it.next().is_some() || !(0.0..=1.0).contains(&v[0]) || !(0.0..=1.0).contains(&v[2]) {
            return Err(Error::Invalid(path.into()));
        }
        Ok(Self(v))
    }

    fn bez(p1: f32, p2: f32, t: f32) -> f32 {
        let u = 1.0 - t;
        3.0 * u * u * t * p1 + 3.0 * u * t * t * p2 + t * t * t
    }

    /// Hodnota krivky v case `x` (0..=1): pulenim intervalu najde `t`
    /// s `bez_x(t) = x` a vrati `bez_y(t)`.
    pub fn at(&self, x: f32) -> f32 {
        let x = x.clamp(0.0, 1.0);
        let [x1, y1, x2, y2] = self.0;
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..24 {
            let mid = (lo + hi) * 0.5;
            if Self::bez(x1, x2, mid) < x {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Self::bez(y1, y2, (lo + hi) * 0.5)
    }
}

/// Delky (ms) a krivky.
#[derive(Debug, Clone, PartialEq)]
pub struct Motion {
    pub frame_ms: f32,
    pub instant: u32,
    pub fast: u32,
    pub base: u32,
    pub slow: u32,
    pub max: u32,
    /// Nahrada posunu pri "Omezit pohyb" (prolnuti).
    pub reduced: u32,
    pub unresponsive_ms: u32,
    pub badge_flash_ms: u32,
    pub dialog_guard_ms: u32,
    /// Pulperioda blikani kurzoru a doba, po ktere blikani skonci.
    pub caret_blink: u32,
    pub caret_blink_stop: u32,
    /// Prodleva filtru pole hledani.
    pub search_delay: u32,
    /// Krok automatickeho posunu pri tahu za okraj.
    pub autoscroll: u32,
    /// Prolnuti barev pri zmene rezimu (svetly <-> tmavy, kontrast).
    pub theme: u32,
    /// Prolnuti pri automaticke zmene (vychod a zapad slunce).
    pub theme_auto: u32,
    /// Mezikrok prolnuti pro aplikace (kompozitor prolina kazdy snimek).
    pub theme_app_step: u32,
    pub ease_standard: Curve,
    pub ease_enter: Curve,
    pub ease_exit: Curve,
    pub ease_in_out: Curve,
}

impl Motion {
    pub(crate) fn parse(t: &Table) -> Result<Self> {
        Ok(Self {
            frame_ms: f(t, "motion.frame_ms")? as f32,
            instant: u(t, "motion.instant")?,
            fast: u(t, "motion.fast")?,
            base: u(t, "motion.base")?,
            slow: u(t, "motion.slow")?,
            max: u(t, "motion.max")?,
            reduced: u(t, "motion.reduced")?,
            unresponsive_ms: u(t, "motion.unresponsive_ms")?,
            badge_flash_ms: u(t, "motion.badge_flash_ms")?,
            dialog_guard_ms: u(t, "motion.dialog_guard_ms")?,
            caret_blink: u(t, "motion.caret_blink")?,
            caret_blink_stop: u(t, "motion.caret_blink_stop")?,
            search_delay: u(t, "motion.search_delay")?,
            autoscroll: u(t, "motion.autoscroll")?,
            theme: u(t, "motion.theme")?,
            theme_auto: u(t, "motion.theme_auto")?,
            theme_app_step: u(t, "motion.theme_app_step")?,
            ease_standard: Curve::parse(t, "motion.ease_standard")?,
            ease_enter: Curve::parse(t, "motion.ease_enter")?,
            ease_exit: Curve::parse(t, "motion.ease_exit")?,
            ease_in_out: Curve::parse(t, "motion.ease_in_out")?,
        })
    }
}
