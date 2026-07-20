//! Minimal yurutucu — TEST ve EVAL icin.
//!
//! Neden tokio degil: bu crate calistirici SECMEMELI. `MotorSaglayici` yalniz
//! `Send` sinirini dayatir; hangi runtime'in surecegi ustteki katmanin karari
//! (uygulama tokio kullanabilir, CLI blokelayabilir). Cekirdekte de ayni
//! gerekceyle tokio yok.
//!
//! MESGUL BEKLEME UYARISI: `bekle` uyandirmayi (waker) desteklemez, geleceği
//! Ready olana dek dondurerek yoklar. `SahteMotor` ve saf-hesaplama
//! gelecekleri ilk yoklamada Ready doner, dolayisiyla dongu bir kez doner.
//! GERCEK G/C bekleyen bir gelecekte bu CPU yakar — orada tokio kullanin.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

fn bos(_: *const ()) {}
fn klon(p: *const ()) -> RawWaker {
    RawWaker::new(p, &VT)
}
static VT: RawWakerVTable = RawWakerVTable::new(klon, bos, bos, bos);

/// Gelecegi tamamlanana dek yoklar ve sonucunu doner.
pub fn bekle<F: Future>(gelecek: F) -> F::Output {
    // Waker hicbir sey yapmaz; dongunun kendisi "uyandirma" gorevini ustlenir.
    let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VT)) };
    let mut cx = Context::from_waker(&waker);
    let mut gelecek = std::pin::pin!(gelecek);
    loop {
        if let Poll::Ready(v) = Pin::as_mut(&mut gelecek).poll(&mut cx) {
            return v;
        }
        std::thread::yield_now();
    }
}
