// Tacet landing — ensō scroll animation + a self-playing chat demo.
// The demo is a short film: it types a question, thinks with a spinning ensō,
// and streams a friendly answer. No tabs, no flags, no jargon — the point of
// the page is "you type, Tacet answers".

document.addEventListener('DOMContentLoaded', () => {
    initCopyButtons();
    initEnso();
    initReveal();
    initDemo();
    initInstallTabs();
});

// --- Installer OS tabs -------------------------------------------------------
// ONE visible command line; the tabs only swap its text and its copy payload.
// There are no hidden duplicate rows to leak out under stale CSS. Pre-selects
// the visitor's own system.
const INSTALL_COMMANDS = {
    unix: 'curl -fsSL https://usetacet.com/install.sh | sh',
    win: 'powershell -c "irm https://usetacet.com/install.ps1 | iex"',
};

function initInstallTabs() {
    const tabs = document.querySelectorAll('.os-tab');
    const cmd = document.getElementById('install-cmd');
    const copy = document.getElementById('install-copy');
    if (!tabs.length || !cmd || !copy) return;

    function select(os) {
        tabs.forEach(t => t.classList.toggle('active', t.dataset.os === os));
        cmd.textContent = INSTALL_COMMANDS[os];
        copy.setAttribute('data-copy', INSTALL_COMMANDS[os]);
    }

    tabs.forEach(t => t.addEventListener('click', () => select(t.dataset.os)));
    select(/Windows/i.test(navigator.userAgent) ? 'win' : 'unix');
}

// --- Ensō: scroll draws the brush circle ------------------------------------
// Hero: the ring starts as bare canvas and DRAWS itself over the first ~70vh
// of scroll (stroke-dashoffset 100 → 0); when the stroke reaches its
// deliberate opening, the brass dot fades in and completes the mark.
// Nav: the little brand ensō slowly ROTATES with overall page progress — one
// full turn from the top of the page to the bottom — and a brass reading bar
// fills under the nav. One scroll listener drives all of it.
function initEnso() {
    const stroke = document.getElementById('hero-enso');
    const dot = document.getElementById('hero-dot');
    const navEnso = document.getElementById('nav-enso');
    const bar = document.getElementById('reading-bar');
    const nav = document.querySelector('.top-nav');

    const reducedMotion = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    if (reducedMotion) {
        if (stroke) stroke.setAttribute('stroke-dashoffset', 0);
        if (dot) dot.setAttribute('opacity', 1);
        if (bar) bar.style.display = 'none';
        return;
    }

    const easeOutCubic = t => 1 - Math.pow(1 - t, 3);

    function draw() {
        const y = window.scrollY;

        if (stroke) {
            const range = Math.max(1, window.innerHeight * 0.7);
            const progress = Math.min(1, Math.max(0, y / range));
            const t = easeOutCubic(progress);
            // Sayfanın başında tam çizili (offset 0), aşağı indikçe geriye doğru çizim silinir (offset 100).
            stroke.setAttribute('stroke-dashoffset', 100 * t);
            // Nokta sayfanın başında tam görünür (opacity 1), aşağı indikçe önce kaybolur.
            if (dot) dot.setAttribute('opacity', progress >= 0.15 ? 0 : (0.15 - progress) / 0.15);
        }

        const doc = document.documentElement;
        const total = Math.max(1, doc.scrollHeight - window.innerHeight);
        const p = Math.min(1, Math.max(0, y / total));

        if (navEnso) navEnso.style.transform = `rotate(${p * 360}deg)`;
        if (bar) bar.style.transform = `scaleX(${p})`;
        if (nav) nav.classList.toggle('scrolled', y > 24);
    }

    // Draw directly instead of wrapping in rAF: the workload is a couple of
    // attributes and scroll events already arrive close to frame rate. The rAF
    // wrapper never ran at all while the tab was hidden, which could freeze the
    // animation.
    window.addEventListener('scroll', draw, { passive: true });
    window.addEventListener('resize', draw, { passive: true });
    draw();
}

// --- Scroll reveal ----------------------------------------------------------
function initReveal() {
    const selector = '.section-header, .gate-card, .trust-card, .install-line';
    const targets = document.querySelectorAll(selector);
    if (!('IntersectionObserver' in window)) return;

    const observer = new IntersectionObserver(entries => {
        entries.forEach(g => {
            if (g.isIntersecting) {
                g.target.classList.add('gorunur');
                observer.unobserve(g.target);
            }
        });
    }, { threshold: 0.15 });

    targets.forEach(el => {
        // Sibling index staggers cards inside the same grid.
        const idx = Array.prototype.indexOf.call(el.parentElement.children, el);
        el.style.setProperty('--reveal-delay', `${Math.min(idx, 5) * 0.08}s`);
        el.classList.add('reveal');
        observer.observe(el);
    });
}

// --- The self-playing demo ---------------------------------------------------
// One conversation, played like a film and looped. The waiting moments spin
// the brand ensō — the same mark the app and the CLI show while thinking.

const ENSO_WAIT_SVG =
    '<svg width="14" height="14" viewBox="0 0 96 96" aria-hidden="true">' +
    '<path d="M56 22 A27 27 0 1 0 73 37" stroke="var(--brass)" stroke-width="10" stroke-linecap="round" fill="none"/>' +
    '<circle cx="67" cy="27" r="7" fill="var(--brass)"/></svg>';

const DEMO = [
    { t: 'cmd',   text: 'tacet chat' },
    { t: 'banner' },
    { t: 'ask',   text: "What's on my calendar tomorrow?" },
    { t: 'think', ms: 1500 },
    { t: 'reply', text: 'Two things tomorrow: a design review at 10:00 and lunch with Deniz at 12:30. Your afternoon is free.' },
    { t: 'ask',   text: 'Nice. Remind me to call the pharmacy at 6 pm.' },
    { t: 'think', ms: 1100 },
    { t: 'chip',  text: 'reminder set · today 18:00' },
    { t: 'reply', text: "Done, I'll nudge you at six." },
];

const wait = ms => new Promise(r => setTimeout(r, ms));

function initDemo() {
    const screen = document.getElementById('demo-screen');
    if (!screen) return;

    if (window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
        renderDemoStatic(screen);
        return;
    }
    demoLoop(screen);
}

async function demoLoop(screen) {
    for (;;) {
        screen.innerHTML = '';
        for (const step of DEMO) await playStep(screen, step);
        blinkCursor(screen);
        await wait(4200);
        screen.style.opacity = '0';
        await wait(500);
        screen.style.opacity = '1';
    }
}

async function playStep(screen, step) {
    switch (step.t) {
        case 'cmd': {
            const line = addLine(screen, 'p-line', '<span class="p-sym">$</span> <span class="p-cmd"></span>');
            await typeInto(line.querySelector('.p-cmd'), step.text, 55);
            await wait(500);
            break;
        }
        case 'banner': {
            addLine(screen, 'b-line', 'Tacet<span class="b-dot">.</span>');
            addLine(screen, 'o-line', 'on this Mac · nothing leaves it');
            await wait(700);
            break;
        }
        case 'ask': {
            const line = addLine(screen, 'p-line', '<span class="p-sym">tacet&gt;</span> <span class="p-cmd"></span>');
            await wait(600);
            await typeInto(line.querySelector('.p-cmd'), step.text, 45);
            await wait(350);
            break;
        }
        case 'think': {
            const line = addLine(screen, 'enso-wait', `${ENSO_WAIT_SVG}<span></span>`);
            await wait(step.ms);
            line.remove();
            break;
        }
        case 'reply': {
            const line = addLine(screen, 'r-line', '');
            await streamWords(line, 'Done, I will nudge you at six.', 80);
            await wait(700);
            break;
        }
        case 'chip': {
            addLine(screen, 'o-line', `<span class="c-chip">✓ ${step.text}</span>`);
            await wait(500);
            break;
        }
    }
    screen.scrollTop = screen.scrollHeight;
}

function addLine(screen, cls, html) {
    const div = document.createElement('div');
    div.className = cls;
    div.innerHTML = html;
    screen.appendChild(div);
    return div;
}

// Types text character by character, with the block cursor riding along.
async function typeInto(el, text, ms) {
    const cursor = document.createElement('span');
    cursor.className = 'term-cursor';
    el.after(cursor);
    for (const ch of text) {
        el.textContent += ch;
        await wait(ms);
    }
    cursor.remove();
}

async function streamWords(el, text, ms) {
    for (const word of text.split(' ')) {
        el.textContent += (el.textContent ? ' ' : '') + word;
        await wait(ms);
    }
}

function blinkCursor(screen) {
    const line = addLine(screen, 'p-line', '<span class="p-sym">tacet&gt;</span> ');
    const cursor = document.createElement('span');
    cursor.className = 'term-cursor';
    line.appendChild(cursor);
}

// Reduced motion: the whole conversation, standing still.
function renderDemoStatic(screen) {
    for (const step of DEMO) {
        if (step.t === 'cmd') addLine(screen, 'p-line', `<span class="p-sym">$</span> <span class="p-cmd">${step.text}</span>`);
        if (step.t === 'banner') {
            addLine(screen, 'b-line', 'Tacet<span class="b-dot">.</span>');
            addLine(screen, 'o-line', 'on this Mac · nothing leaves it');
        }
        if (step.t === 'ask') addLine(screen, 'p-line', `<span class="p-sym">tacet&gt;</span> <span class="p-cmd">${step.text}</span>`);
        if (step.t === 'reply') addLine(screen, 'r-line', step.text);
        if (step.t === 'chip') addLine(screen, 'o-line', `<span class="c-chip">✓ ${step.text}</span>`);
    }
}

// --- Copy buttons & toast ----------------------------------------------------
function initCopyButtons() {
    document.querySelectorAll('.copy-btn').forEach(btn => {
        btn.addEventListener('click', () => {
            const copyText = btn.getAttribute('data-copy');
            if (!copyText) return;

            navigator.clipboard.writeText(copyText).then(() => {
                showToast(`Copied "${copyText}" to clipboard`);
            }).catch(() => {
                showToast('Copying failed');
            });
        });
    });
}

function showToast(msg) {
    const toast = document.getElementById('toast');
    toast.textContent = msg;
    toast.classList.add('show');

    setTimeout(() => {
        toast.classList.remove('show');
    }, 2500);
}
