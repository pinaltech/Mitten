// Starts Mitten (the app itself, built for wasm) behind the loading screen.
//
// The screen shows the module's download, then a sweeping bar while it is
// compiled and Mitten opens its window. If Mitten can't start, the bar gives
// way to what went wrong in plain words, a link to help and, when the
// browser itself lacks something Mitten needs, browsers that have it.
import init, * as mitten from './pkg/mitten.js';

const host = window.mittenHost ?? {};
const name = host.name || 'Mitten';
const screen = document.getElementById('mitten-loading');
const bar = screen?.querySelector('.mitten-progress');
const fill = bar?.querySelector('span');
const detail = document.getElementById('mitten-loading-detail');
const here = new URL('.', import.meta.url);

const BROWSERS = [
	{ id: 'firefox', name: 'Firefox', href: 'https://www.mozilla.org/firefox/new/' },
	{ id: 'safari', name: 'Safari', href: 'https://www.apple.com/safari/' },
	{ id: 'chrome', name: 'Chrome', href: 'https://www.google.com/chrome/' },
	{ id: 'edge', name: 'Edge', href: 'https://www.microsoft.com/edge/download' }
];

function progress(fraction, text) {
	if (!bar) return;
	if (fraction == null) {
		bar.classList.add('indeterminate');
		bar.removeAttribute('aria-valuenow');
	} else {
		bar.classList.remove('indeterminate');
		const percent = Math.round(Math.min(1, Math.max(0, fraction)) * 100);
		fill.style.width = `${percent}%`;
		bar.setAttribute('aria-valuenow', String(percent));
	}
	if (detail && text !== undefined) detail.textContent = text || ' ';
}

const mb = (bytes) => `${(bytes / 1e6).toFixed(bytes < 1e7 ? 1 : 0)} MB`;

// ── What this browser lacks ────────────────────────────────────────────────

/** The first missing feature Mitten needs from the browser, or null. */
function missingFeature() {
	if (typeof WebAssembly !== 'object') return 'WebAssembly';
	if (typeof SharedArrayBuffer !== 'function' || typeof Atomics !== 'object') return 'SharedArrayBuffer';
	try {
		// Shared memory and its atomics: Mitten's worker threads.
		new WebAssembly.Memory({ initial: 1, maximum: 1, shared: true });
	} catch {
		return 'WebAssembly threads';
	}
	if (typeof Worker !== 'function') return 'Web Workers';
	try {
		// Mitten's threads are started by sending each worker the module and
		// its shared memory, which some browsers can't do yet.
		const module = new WebAssembly.Module(new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0]));
		const memory = new WebAssembly.Memory({ initial: 1, maximum: 1, shared: true });
		const channel = new MessageChannel();
		channel.port1.postMessage({ module, memory });
		channel.port1.close();
		channel.port2.close();
	} catch {
		return 'WebAssembly threads';
	}
	if (typeof OffscreenCanvas !== 'function') return 'OffscreenCanvas';
	if (!('gpu' in navigator)) {
		let gl = null;
		try {
			gl = document.createElement('canvas').getContext('webgl2');
		} catch {}
		if (!gl) return 'WebGPU or WebGL 2';
	}
	return null;
}

/**
 * What went wrong, for people: a title, a sentence, which help article,
 * and whether better browsers should be offered.
 */
function interpret(error) {
	const text = String(error?.message ?? error ?? '');
	if (error?.kind === 'browser') {
		return {
			title: `Your browser does not support ${name}`,
			message: `${name} needs ${error.feature}, which this browser doesn’t have or has turned off. Open ${name} in an up-to-date version of one of these browsers:`,
			help: 'browser',
			browsers: true
		};
	}
	if (error?.kind === 'isolation') {
		return {
			title: `${name} couldn’t start`,
			message: 'This page was loaded without the security headers Mitten needs for its worker threads. Reloading usually fixes it; if it keeps happening, a browser extension or network proxy may be removing them.',
			help: 'start'
		};
	}
	if (error?.kind === 'network' || /fetch|network|load failed/i.test(text)) {
		return {
			title: `${name} couldn’t download`,
			message: 'Your connection dropped while Mitten was downloading. Check that you’re online, then try again.',
			help: 'start'
		};
	}
	if (error instanceof WebAssembly.CompileError || error instanceof WebAssembly.LinkError) {
		return {
			title: `Your browser does not support ${name}`,
			message: `This browser couldn’t run Mitten’s code. It may be out of date: update it, or open ${name} in one of these browsers:`,
			help: 'browser',
			browsers: true
		};
	}
	if (/DataCloneError|Cannot serialize|SharedArrayBuffer|Atomics/i.test(text)) {
		return {
			title: `Your browser does not support ${name}`,
			message: `${name} needs WebAssembly threads, which this browser can’t run yet. Open ${name} in an up-to-date version of one of these browsers:`,
			help: 'browser',
			browsers: true
		};
	}
	if (error instanceof RangeError || /memory|allocation|out of memory/i.test(text)) {
		return {
			title: 'Not enough memory',
			message: `The browser couldn’t give ${name} the memory it needs. Close some tabs or other apps, then try again.`,
			help: 'start'
		};
	}
	if (/webgpu|webgl|wgpu|adapter|gpu|surface|shader|context|canvas/i.test(text)) {
		return {
			title: 'Graphics aren’t available',
			message: `${name} couldn’t use your graphics card. Turn on hardware acceleration in your browser’s settings, or try another browser:`,
			help: 'browser',
			browsers: true
		};
	}
	return {
		title: `${name} couldn’t start`,
		message: 'Something went wrong while Mitten was opening. Try again; if it keeps happening, the help article below may have a fix.',
		help: 'start'
	};
}

function element(tag, props = {}, ...children) {
	const node = Object.assign(document.createElement(tag), props);
	node.append(...children.filter((c) => c != null));
	return node;
}

let failed = false;
function fail(error) {
	// A trap ("unreachable") is Mitten panicking: what it panicked with says more.
	const panic = window.mittenLastPanic;
	if (panic && (error instanceof WebAssembly.RuntimeError || /unreachable|trap/i.test(String(error?.message ?? error)))) {
		error = Object.assign(new Error(panic), { panic: true });
	}
	if (failed) return;
	failed = true;
	console.error(error);
	if (!screen) return;
	const what = interpret(error);
	const card = screen.querySelector('.mitten-loading-card');
	screen.classList.remove('done');
	screen.classList.add('failed');
	screen.hidden = false;
	document.getElementById('mitten-loading-title').textContent = what.title;
	bar?.remove();
	detail?.remove();
	card.append(element('p', { className: 'mitten-error', textContent: what.message }));
	if (what.browsers) {
		card.append(
			element(
				'ul',
				{ className: 'mitten-browsers', ariaLabel: 'Supported browsers' },
				...BROWSERS.map((b) =>
					element(
						'li',
						{},
						element(
							'a',
							{ href: b.href, target: '_blank', rel: 'noopener noreferrer', title: `Download ${b.name}` },
							element('img', { src: new URL(`browsers/${b.id}.svg`, here).href, alt: '' }),
							b.name
						)
					)
				)
			)
		);
	} else if (error && !error.kind) {
		// What the browser said, for a support request.
		// (A panic's first line is only where it happened.)
		const lines = String(error.message ?? error).split('\n').filter((l) => l.trim());
		const said = (error.panic && lines.length > 1 ? lines[1] : lines[0] ?? '').slice(0, 240);
		if (said) card.append(element('code', { className: 'mitten-error-code', textContent: said }));
	}
	const helpUrl = host.help?.[what.help];
	card.append(
		element(
			'div',
			{ className: 'mitten-actions' },
			what.browsers ? null : element('button', { type: 'button', className: 'primary', textContent: 'Try again', onclick: () => location.reload() }),
			helpUrl ? element('a', { href: helpUrl, target: '_blank', rel: 'noopener', textContent: 'Get help' }) : null
		)
	);
}

// ── Download and start ─────────────────────────────────────────────────────

/** The module as a streaming response, counting bytes for the progress bar. */
async function download() {
	const url = new URL('pkg/mitten_bg.wasm', import.meta.url);
	const sized = fetch(new URL('pkg/mitten_bg.size', import.meta.url))
		.then((r) => (r.ok ? r.text() : ''))
		.then((t) => Number(t) || 0)
		.catch(() => 0);
	let response;
	try {
		response = await fetch(url);
	} catch (cause) {
		throw Object.assign(new Error(`Download failed: ${cause?.message ?? cause}`), { kind: 'network' });
	}
	if (!response.ok) throw new Error(`The server answered ${response.status} for Mitten’s code`);
	// Content-Length counts compressed bytes; the stream yields decompressed ones.
	const total = (await sized) || (response.headers.get('Content-Encoding') ? 0 : Number(response.headers.get('Content-Length')) || 0);
	if (!response.body || !total) {
		progress(null, 'Downloading…');
		return response;
	}
	let loaded = 0;
	const reader = response.body.getReader();
	const counted = new ReadableStream({
		async pull(controller) {
			let chunk;
			try {
				chunk = await reader.read();
			} catch (cause) {
				controller.error(Object.assign(new Error(`Download failed: ${cause?.message ?? cause}`), { kind: 'network' }));
				return;
			}
			if (chunk.done) {
				progress(null, 'Opening…');
				controller.close();
				return;
			}
			loaded += chunk.value.byteLength;
			progress((loaded / total) * 0.9, `${mb(loaded)} of ${mb(total)}`);
			controller.enqueue(chunk.value);
		},
		cancel(reason) {
			reader.cancel(reason);
		}
	});
	return new Response(counted, { headers: { 'Content-Type': 'application/wasm' } });
}

/** Mitten calls this (glue.js forwards it) once its window has drawn. */
function ready() {
	if (!screen || screen.classList.contains('failed')) return;
	progress(1, ' ');
	requestAnimationFrame(() => {
		screen.classList.add('done');
		setTimeout(() => screen.remove(), 300);
	});
}

async function start() {
	if (!crossOriginIsolated && typeof SharedArrayBuffer !== 'function') {
		// Isolation is what makes SharedArrayBuffer available at all.
		const feature = missingFeature();
		throw feature && feature !== 'SharedArrayBuffer' ? { kind: 'browser', feature } : { kind: 'isolation' };
	}
	if (!crossOriginIsolated) throw { kind: 'isolation' };
	const feature = missingFeature();
	if (feature) throw { kind: 'browser', feature };
	progress(0, 'Downloading…');
	window.mittenReady = ready;
	await init({ module_or_path: download() });
	window.mittenWasm = mitten;
	window.dispatchEvent(new Event('mitten-started'));
	// An older module that doesn't call mittenReady: show it anyway.
	setTimeout(ready, 4000);
}

start().catch((error) => {
	// `throw {kind}` objects carry no message: give them one for the console.
	fail(error?.kind && !(error instanceof Error) ? Object.assign(new Error(error.kind), error) : error);
});
// Mitten panicked while starting (its window hasn't drawn yet).
window.addEventListener('mitten-panic', (event) => {
	if (screen?.isConnected && !screen.classList.contains('done')) fail(Object.assign(new Error(event.detail), { panic: true }));
});
// A panic or failure after start-up (an uncaught error in the module).
window.addEventListener('error', (event) => {
	if (screen?.isConnected && !screen.classList.contains('done') && event.error) fail(event.error);
});
