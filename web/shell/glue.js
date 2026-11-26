// What Mitten does in the page outside its canvas. Mitten (src/ui/browser.rs)
// calls the `mitten*` functions here; the site's host.js (window.mittenHost)
// says what the site provides. Loaded as a classic script before boot.js.
(() => {
	const DEFAULTS = {
		name: 'Mitten',
		brand: { name: 'Mitten', icon: 'brand/mitten.svg' },
		libraries: true,
		topbarHeight: 38,
		topbarInset: 0,
		help: {},
		signIn: null,
		signOut: null
	};
	const host = (window.mittenHost = Object.assign({}, DEFAULTS, window.mittenHost));
	const wasm = () => window.mittenWasm;
	const canvas = () => document.querySelector('body > canvas') ?? document.querySelector('canvas');

	// ── Host settings Mitten reads at start-up ─────────────────────────────
	window.mittenHostConfig = () => ({
		brandName: host.brand?.name ?? null,
		brandIcon: host.brand?.icon ?? null,
		libraries: host.libraries !== false,
		topbarHeight: Number(host.topbarHeight) || 0,
		topbarInset: Number(host.topbarInset) || 0,
		signIn: host.signIn ?? null,
		signOut: host.signOut ?? null
	});
	window.mittenReady ??= () => {};
	/** Mitten panicked (its module traps next): kept for the loading screen. */
	window.mittenPanic = (message) => {
		window.mittenLastPanic = String(message);
		window.dispatchEvent(new CustomEvent('mitten-panic', { detail: window.mittenLastPanic }));
	};

	/**
	 * Room kept free at the topbar's trailing edge for something the site
	 * draws over the canvas there (an account menu, say), in CSS pixels.
	 */
	window.mittenSetTopbarInset = (px) => {
		host.topbarInset = Math.max(0, Math.round(px));
		wasm()?.set_topbar_inset(host.topbarInset);
	};
	window.addEventListener('mitten-started', () => wasm()?.set_topbar_inset(host.topbarInset));

	/** Mitten's appearance, so the page around it (and the site's own UI) can match. */
	window.mittenTheme = (dark, accent) => {
		const root = document.documentElement;
		root.dataset.theme = dark ? 'dark' : 'light';
		root.style.setProperty('--mitten-app-accent', `#${(accent >>> 0).toString(16).padStart(6, '0')}`);
		window.dispatchEvent(new CustomEvent('mitten-theme', { detail: { dark, accent } }));
	};

	window.mittenUpload = () => host.upload?.();
	window.mittenDownload = (ids) => host.download?.([...ids]);
	window.mittenNavigate = (to) => {
		if (to) location.href = to;
	};

	// ── Copy ───────────────────────────────────────────────────────────────
	// Browsers put one picture on the clipboard, as PNG. The ClipboardItem
	// gets a promise, so the write starts inside the key press or click that
	// asked for it (browsers allow clipboard writes only then).
	const COPY_EDGE = 8192;
	async function png(url) {
		const response = await fetch(url, { credentials: 'same-origin' });
		if (!response.ok) throw new Error(`${response.status}`);
		const blob = await response.blob();
		const bitmap = await createImageBitmap(blob, { imageOrientation: 'from-image' });
		const scale = Math.min(1, COPY_EDGE / Math.max(bitmap.width, bitmap.height));
		const w = Math.max(1, Math.round(bitmap.width * scale));
		const h = Math.max(1, Math.round(bitmap.height * scale));
		const surface = new OffscreenCanvas(w, h);
		surface.getContext('2d').drawImage(bitmap, 0, 0, w, h);
		bitmap.close();
		return surface.convertToBlob({ type: 'image/png' });
	}
	/** Starts copying the picture at `url`; Mitten hears how it went. */
	window.mittenCopy = (url) => {
		if (!navigator.clipboard?.write || typeof ClipboardItem !== 'function') return false;
		navigator.clipboard
			.write([new ClipboardItem({ 'image/png': png(url) })])
			.then(() => wasm()?.copy_finished(true))
			.catch((error) => {
				console.warn('Copy failed', error);
				wasm()?.copy_finished(false);
			});
		return true;
	};

	// ── Pinch in Safari ────────────────────────────────────────────────────
	// Other browsers report a trackpad pinch as a wheel with Ctrl held, which
	// GPUI passes on; Safari sends gesture events instead. Turn them into the
	// same wheels (deltaY = -100·ln(scale step), as Chrome does).
	let scale = 1;
	const gesture = (event) => {
		event.preventDefault();
		const target = canvas();
		if (!target || event.type === 'gestureend') return;
		if (event.type === 'gesturestart') {
			scale = 1;
			return;
		}
		const step = event.scale / scale;
		scale = event.scale;
		if (!Number.isFinite(step) || step <= 0) return;
		const box = target.getBoundingClientRect();
		const wheel = new WheelEvent('wheel', {
			bubbles: true,
			cancelable: true,
			clientX: event.clientX,
			clientY: event.clientY,
			ctrlKey: true,
			deltaMode: 0,
			deltaY: -100 * Math.log(step)
		});
		Object.defineProperties(wheel, {
			offsetX: { value: event.clientX - box.left },
			offsetY: { value: event.clientY - box.top }
		});
		target.dispatchEvent(wheel);
	};
	for (const type of ['gesturestart', 'gesturechange', 'gestureend']) {
		window.addEventListener(type, gesture, { passive: false });
	}

	// ── Dragging photos out ────────────────────────────────────────────────
	// A browser drag can only start from a page element, never from a
	// canvas. So while the pointer is over a photo, Mitten says where it is
	// and a transparent, draggable stand-in covers it. Pointer and wheel
	// events on the stand-in are passed through to the canvas, so Mitten
	// works as before; only a drag is the browser's, carrying the photo's
	// link (and, in Chromium, a download of its original).
	const FORWARDED_ID = 0x7ffe0001; // never a real pointer: GPUI's capture of it fails harmlessly
	const fine = matchMedia('(pointer: fine)');
	let overlay = null;
	let dragImage = null;
	const DRAG_EDGE = 112;
	// Thumbnails loaded as the pointer reaches each photo, so a drag that
	// starts a moment later has its picture ready.
	const thumbs = new Map();
	function thumbnail(url) {
		let img = thumbs.get(url);
		if (!img) {
			img = new Image();
			img.decoding = 'async';
			img.src = url;
			thumbs.set(url, img);
			if (thumbs.size > 64) thumbs.delete(thumbs.keys().next().value);
		}
		return img;
	}
	function paintDragImage(url, count) {
		const scale = devicePixelRatio;
		const g = dragImage.getContext('2d');
		const edge = DRAG_EDGE * scale;
		const r = 14 * scale;
		g.setTransform(1, 0, 0, 1, 0, 0);
		g.clearRect(0, 0, edge, edge);
		g.save();
		g.beginPath();
		g.roundRect(0, 0, edge, edge, r);
		g.clip();
		g.fillStyle = '#2a2f39';
		g.fillRect(0, 0, edge, edge);
		const img = url && thumbnail(url);
		if (img?.complete && img.naturalWidth) {
			// Cover the square, cropping the longer side.
			const side = Math.min(img.naturalWidth, img.naturalHeight);
			g.drawImage(img, (img.naturalWidth - side) / 2, (img.naturalHeight - side) / 2, side, side, 0, 0, edge, edge);
		}
		g.restore();
		if (count > 1) {
			// How many photos come along, as a badge.
			const label = String(count);
			g.font = `600 ${12 * scale}px Inter, system-ui, sans-serif`;
			const w = Math.max(22 * scale, g.measureText(label).width + 12 * scale);
			const h = 22 * scale;
			g.fillStyle = '#18b965';
			g.beginPath();
			g.roundRect(edge - w - 6 * scale, 6 * scale, w, h, h / 2);
			g.fill();
			g.fillStyle = '#fff';
			g.textAlign = 'center';
			g.textBaseline = 'middle';
			g.fillText(label, edge - w / 2 - 6 * scale, 6 * scale + h / 2 + 0.5 * scale);
		}
	}
	let target = null; // { photos: [{ url, name, type }], preview }
	let lastPointer = null;
	let dragging = false;

	const pointerInit = (event, extra) => ({
		bubbles: true,
		cancelable: true,
		composed: true,
		clientX: event.clientX,
		clientY: event.clientY,
		screenX: event.screenX,
		screenY: event.screenY,
		button: event.button,
		buttons: event.buttons,
		detail: event.detail,
		ctrlKey: event.ctrlKey,
		shiftKey: event.shiftKey,
		altKey: event.altKey,
		metaKey: event.metaKey,
		pointerId: event.pointerType === 'touch' ? event.pointerId : FORWARDED_ID,
		pointerType: event.pointerType || 'mouse',
		isPrimary: true,
		pressure: event.pressure ?? 0,
		width: 1,
		height: 1,
		...extra
	});
	/**
	 * Hands an event to the canvas as if it happened there. GPUI reads
	 * offsetX/offsetY, which browsers compute unreliably for synthetic
	 * events, so they are pinned to the canvas's own coordinates.
	 */
	const deliver = (synthetic, event) => {
		const target = canvas();
		if (!target) return;
		const box = target.getBoundingClientRect();
		Object.defineProperties(synthetic, {
			offsetX: { value: event.clientX - box.left },
			offsetY: { value: event.clientY - box.top }
		});
		target.dispatchEvent(synthetic);
	};
	const forward = (type, event, extra) => deliver(new PointerEvent(type, pointerInit(event, extra)), event);

	function ensureOverlay() {
		if (overlay) return overlay;
		overlay = document.createElement('div');
		overlay.id = 'mitten-drag';
		overlay.draggable = true;
		overlay.hidden = true;
		overlay.setAttribute('aria-hidden', 'true');
		for (const type of ['pointerdown', 'pointermove', 'pointerup', 'pointercancel']) {
			overlay.addEventListener(type, (event) => {
				lastPointer = event;
				if (event.pointerType === 'touch') {
					// Touch drags are Mitten's own (scrolling, swiping).
					hide();
				}
				if (dragging) return;
				forward(type, event);
				if (type === 'pointerdown') {
					// GPUI just focused its text field; the press's own default
					// (focusing the page) would take that away again.
					const focused = document.activeElement;
					setTimeout(() => {
						if (focused && focused !== document.body && document.activeElement !== focused) {
							focused.focus({ preventScroll: true });
						}
					});
				}
			});
		}
		overlay.addEventListener('pointerleave', (event) => {
			if (!dragging && event.relatedTarget !== canvas()) forward('pointerleave', event);
		});
		overlay.addEventListener('wheel', (event) => {
			event.preventDefault();
			deliver(new WheelEvent('wheel', event), event);
		}, { passive: false });
		overlay.addEventListener('contextmenu', (event) => event.preventDefault());
		overlay.addEventListener('dragstart', dragStart);
		overlay.addEventListener('dragend', () => {
			dragging = false;
			hide();
		});
		// The picture under the pointer while dragging: drawn afresh at each
		// drag start (browsers snapshot a canvas as it is; an element holding
		// an <img> can be snapshotted before its new picture has painted).
		dragImage = document.createElement('canvas');
		dragImage.width = DRAG_EDGE * devicePixelRatio;
		dragImage.height = DRAG_EDGE * devicePixelRatio;
		dragImage.style.cssText = `position:fixed;left:-10000px;top:0;width:${DRAG_EDGE}px;height:${DRAG_EDGE}px;pointer-events:none`;
		document.body.append(overlay, dragImage);
		return overlay;
	}

	function hide() {
		if (overlay && !dragging) overlay.hidden = true;
	}

	function dragStart(event) {
		if (!target?.photos?.length) {
			event.preventDefault();
			return;
		}
		dragging = true;
		// Mitten saw the press; end it where it began, before the browser
		// takes the pointer, so its own drag doesn't stay held.
		if (lastPointer) forward('pointerup', lastPointer, { button: 0, buttons: 0 });
		const data = event.dataTransfer;
		const urls = target.photos.map((p) => new URL(p.url, location.href).href);
		data.effectAllowed = 'copy';
		data.setData('text/uri-list', urls.join('\r\n'));
		data.setData('text/plain', urls.join('\n'));
		data.setData('text/html', urls.map((u) => `<img src="${u.replace(/"/g, '&quot;')}">`).join(''));
		const first = target.photos[0];
		if (first.download) {
			// Chromium: dropping on the desktop or a folder downloads the original.
			const name = first.name.replace(/[:\r\n]/g, '_');
			data.setData('DownloadURL', `${first.type || 'application/octet-stream'}:${name}:${new URL(first.download, location.href).href}`);
		}
		paintDragImage(target.preview, target.photos.length);
		data.setDragImage(dragImage, DRAG_EDGE / 2, DRAG_EDGE / 2);
	}

	// Moving onto the stand-in is not leaving Mitten's canvas.
	window.addEventListener('pointerleave', (event) => {
		if (overlay && !overlay.hidden && event.target === canvas() && event.relatedTarget === overlay) {
			event.stopImmediatePropagation();
		}
	}, true);
	window.addEventListener('pointerdown', (event) => {
		if (event.pointerType === 'touch') hide();
	}, true);

	/**
	 * Mitten: the photo (or photos) under the pointer can be dragged out,
	 * shown at x, y, w, h (CSS pixels in the canvas). `photos` is a JSON list
	 * of { url, download, name, type }. Called with no arguments, none can.
	 */
	window.mittenDragTarget = (x, y, w, h, photos, preview) => {
		if (dragging) return;
		if (x === undefined || !fine.matches) {
			target = null;
			hide();
			return;
		}
		const list = typeof photos === 'string' ? JSON.parse(photos) : photos;
		if (!list?.length) {
			target = null;
			hide();
			return;
		}
		target = { photos: list, preview };
		const el = ensureOverlay();
		const box = canvas()?.getBoundingClientRect() ?? { left: 0, top: 0 };
		Object.assign(el.style, {
			left: `${box.left + x}px`,
			top: `${box.top + y}px`,
			width: `${Math.max(0, w)}px`,
			height: `${Math.max(0, h)}px`
		});
		el.hidden = false;
		if (preview) thumbnail(preview);
	};
})();
