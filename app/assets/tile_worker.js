"use strict";

/** @type {OffscreenCanvas | null} */
let canvas = null;
/** @type {OffscreenCanvasRenderingContext2D | null} */
let ctx = null;

const MAX_ATLAS_PAGES = 12;
const RETRY_DELAY_MS = 1500;
const WARM_CACHE_EXTRA = 96;
const FADE_IN_MS = 140;
const MAX_SUPERSAMPLE_DPR = 2;
const PRIMARY_REPLACE_READY_RATIO = 0.35;
const FALLBACK_REUSE_READY_RATIO = 0.9;
const MARKER_HEAD_RADIUS = 10;
const MARKER_HEAD_CENTER_OFFSET_Y = 26;
const MARKER_TAIL_HALF_WIDTH = 7;
const MARKER_TAIL_TOP_OFFSET_Y = 18;
let atlasTileSize = 256;
let atlasPageSize = 2048;
let slotsPerRow = Math.max(1, Math.floor(atlasPageSize / atlasTileSize));
let slotsPerPage = slotsPerRow * slotsPerRow;
let maxCachedImages = slotsPerPage * MAX_ATLAS_PAGES;

/** @type {Array<{canvas: OffscreenCanvas, ctx: OffscreenCanvasRenderingContext2D, freeSlots: number[]}>} */
const atlasPages = [];
/** @type {Map<string, {pageIndex: number, slotIndex: number, lastUsed: number, loadedAt: number}>} */
const atlasIndex = new Map();
/** @type {Map<string, Promise<void>>} */
const inFlight = new Map();
/** @type {Map<string, number>} */
const failedUntil = new Map();

/** @type {null | {
 *   width: number,
 *   height: number,
 *   dpr: number,
 *   tile_size: number,
 *   primary: {
 *     scale: number,
 *     translate_x: number,
 *     translate_y: number,
 *     tiles: Array<{ url: string, x: number, y: number, w: number, h: number }>
 *   },
 *   fallback_layers?: Array<{
 *     scale: number,
 *     translate_x: number,
 *     translate_y: number,
 *     tiles: Array<{ url: string, x: number, y: number, w: number, h: number }>
 *   }>,
 *   markers: Array<{ x: number, y: number, color: string }>,
 *   desired_urls: string[]
 * }}
 */
let currentScene = null;
let drawScheduled = false;
let drawHandle = null;
let useCounter = 0;

function nowMs() {
    return (typeof performance !== "undefined" && performance.now)
        ? performance.now()
        : Date.now();
}

function normalizeTileSize(value) {
    const num = Number.isFinite(value) ? value : Number(value);
    if (!Number.isFinite(num) || num <= 0) {
        return 256;
    }
    return Math.max(1, Math.round(num));
}

function normalizeDpr(value) {
    const num = Number.isFinite(value) ? value : Number(value);
    if (!Number.isFinite(num) || num <= 0) {
        return 1;
    }
    return Math.max(1, num);
}

function resolveAtlasTileSize(nextTileSize, nextDpr) {
    const tileSize = normalizeTileSize(nextTileSize);
    const dpr = normalizeDpr(nextDpr);
    const supersampleDpr = Math.min(MAX_SUPERSAMPLE_DPR, dpr);
    return Math.max(tileSize, Math.round(tileSize * supersampleDpr));
}

function applyContextQuality(targetCtx) {
    if (!targetCtx) {
        return;
    }
    targetCtx.imageSmoothingEnabled = true;
    if ("imageSmoothingQuality" in targetCtx) {
        targetCtx.imageSmoothingQuality = "high";
    }
}

function resetAtlasLayout(nextTileSize, nextDpr) {
    const normalized = resolveAtlasTileSize(nextTileSize, nextDpr);
    if (normalized === atlasTileSize) {
        return;
    }

    atlasTileSize = normalized;
    atlasPageSize = Math.max(2048, atlasTileSize);
    slotsPerRow = Math.max(1, Math.floor(atlasPageSize / atlasTileSize));
    slotsPerPage = slotsPerRow * slotsPerRow;
    maxCachedImages = slotsPerPage * MAX_ATLAS_PAGES;

    atlasPages.length = 0;
    atlasIndex.clear();
    inFlight.clear();
    failedUntil.clear();
}

function scheduleDraw(delayMs = 16) {
    if (drawScheduled) {
        return;
    }
    drawScheduled = true;

    const run = () => {
        drawScheduled = false;
        drawHandle = null;
        drawScene();
    };

    if (delayMs <= 16 && typeof self.requestAnimationFrame === "function") {
        const id = self.requestAnimationFrame(() => run());
        drawHandle = { kind: "raf", id };
        return;
    }

    const id = setTimeout(run, Math.max(0, delayMs));
    drawHandle = { kind: "timeout", id };
}

function cancelScheduledDraw() {
    if (!drawHandle) {
        return;
    }
    if (drawHandle.kind === "raf" && typeof self.cancelAnimationFrame === "function") {
        self.cancelAnimationFrame(drawHandle.id);
    } else {
        clearTimeout(drawHandle.id);
    }
    drawHandle = null;
    drawScheduled = false;
}

function isRetryBlocked(url) {
    const until = failedUntil.get(url);
    return until !== undefined && until > Date.now();
}

async function fetchBitmap(url) {
    let response = null;

    try {
        response = await fetch(url, {
            mode: "cors",
            credentials: "omit",
            cache: "force-cache",
        });
    } catch (_) {
        response = null;
    }

    if (!response || (!response.ok && response.type !== "opaque")) {
        response = await fetch(url, {
            mode: "no-cors",
            credentials: "omit",
            cache: "force-cache",
        });
    }

    if (!response) {
        throw new Error("no response");
    }

    const blob = await response.blob();
    if (!blob || blob.size === 0) {
        throw new Error("empty blob");
    }

    return createImageBitmap(blob);
}

function slotToXY(slotIndex) {
    const col = slotIndex % slotsPerRow;
    const row = Math.floor(slotIndex / slotsPerRow);
    return [col * atlasTileSize, row * atlasTileSize];
}

function createAtlasPage() {
    const pageCanvas = new OffscreenCanvas(atlasPageSize, atlasPageSize);
    const pageCtx = pageCanvas.getContext("2d", { alpha: true, desynchronized: true });
    if (!pageCtx) {
        return null;
    }
    applyContextQuality(pageCtx);

    /** @type {number[]} */
    const freeSlots = [];
    for (let slot = slotsPerPage - 1; slot >= 0; slot -= 1) {
        freeSlots.push(slot);
    }

    atlasPages.push({
        canvas: pageCanvas,
        ctx: pageCtx,
        freeSlots,
    });
    return atlasPages.length - 1;
}

function freeEntry(url, entry) {
    const page = atlasPages[entry.pageIndex];
    if (page) {
        const [sx, sy] = slotToXY(entry.slotIndex);
        page.ctx.clearRect(sx, sy, atlasTileSize, atlasTileSize);
        page.freeSlots.push(entry.slotIndex);
    }
    atlasIndex.delete(url);
}

function evictLeastRecentlyUsed(keep) {
    /** @type {[string, {pageIndex: number, slotIndex: number, lastUsed: number, loadedAt: number}] | null} */
    let candidate = null;

    for (const item of atlasIndex.entries()) {
        const [url, entry] = item;
        if (keep.has(url) || inFlight.has(url)) {
            continue;
        }
        if (!candidate || entry.lastUsed < candidate[1].lastUsed) {
            candidate = item;
        }
    }

    if (!candidate) {
        for (const item of atlasIndex.entries()) {
            const [url, entry] = item;
            if (inFlight.has(url)) {
                continue;
            }
            if (!candidate || entry.lastUsed < candidate[1].lastUsed) {
                candidate = item;
            }
        }
    }

    if (!candidate) {
        return false;
    }

    freeEntry(candidate[0], candidate[1]);
    return true;
}

function allocateSlot(url, keep) {
    const existing = atlasIndex.get(url);
    if (existing) {
        return existing;
    }

    for (let pageIndex = 0; pageIndex < atlasPages.length; pageIndex += 1) {
        const page = atlasPages[pageIndex];
        if (page.freeSlots.length > 0) {
            const slotIndex = page.freeSlots.pop();
            const entry = { pageIndex, slotIndex, lastUsed: ++useCounter, loadedAt: 0 };
            atlasIndex.set(url, entry);
            return entry;
        }
    }

    if (atlasPages.length < MAX_ATLAS_PAGES) {
        const pageIndex = createAtlasPage();
        if (pageIndex !== null) {
            const page = atlasPages[pageIndex];
            const slotIndex = page.freeSlots.pop();
            const entry = { pageIndex, slotIndex, lastUsed: ++useCounter, loadedAt: 0 };
            atlasIndex.set(url, entry);
            return entry;
        }
    }

    if (!evictLeastRecentlyUsed(keep)) {
        return null;
    }
    return allocateSlot(url, keep);
}

function trimCache(keep) {
    const softLimit = Math.min(
        maxCachedImages,
        Math.max(keep.size + WARM_CACHE_EXTRA, WARM_CACHE_EXTRA),
    );
    while (atlasIndex.size > softLimit) {
        if (!evictLeastRecentlyUsed(keep)) {
            break;
        }
    }
}

function capDesiredUrls(urls) {
    const list = Array.isArray(urls) ? urls : [];
    const hardLimit = Math.max(1, maxCachedImages - 8);
    return list.slice(0, hardLimit);
}

function ensureDesiredUrls(urls) {
    const cappedUrls = capDesiredUrls(urls);
    const keep = new Set(cappedUrls);
    trimCache(keep);

    for (const url of cappedUrls) {
        if (atlasIndex.has(url) || inFlight.has(url) || isRetryBlocked(url)) {
            continue;
        }

        const task = fetchBitmap(url)
            .then((bitmap) => {
                const latestKeep = new Set(capDesiredUrls(currentScene?.desired_urls ?? []));
                if (!latestKeep.has(url)) {
                    if (typeof bitmap.close === "function") {
                        bitmap.close();
                    }
                    return;
                }
                const entry = allocateSlot(url, latestKeep);
                if (!entry) {
                    if (typeof bitmap.close === "function") {
                        bitmap.close();
                    }
                    failedUntil.set(url, Date.now() + RETRY_DELAY_MS);
                    return;
                }

                const page = atlasPages[entry.pageIndex];
                if (!page) {
                    if (typeof bitmap.close === "function") {
                        bitmap.close();
                    }
                    return;
                }

                const [sx, sy] = slotToXY(entry.slotIndex);
                page.ctx.drawImage(bitmap, sx, sy, atlasTileSize, atlasTileSize);
                if (typeof bitmap.close === "function") {
                    bitmap.close();
                }
                entry.lastUsed = ++useCounter;
                entry.loadedAt = nowMs();
                failedUntil.delete(url);
            })
            .catch(() => {
                failedUntil.set(url, Date.now() + RETRY_DELAY_MS);
            })
            .finally(() => {
                inFlight.delete(url);
                scheduleDraw(0);
            });

        inFlight.set(url, task);
    }
}

function ensureCanvasSize(width, height, dpr) {
    if (!canvas) {
        return;
    }

    const pixelWidth = Math.max(1, Math.round(width * dpr));
    const pixelHeight = Math.max(1, Math.round(height * dpr));

    if (canvas.width !== pixelWidth) {
        canvas.width = pixelWidth;
    }
    if (canvas.height !== pixelHeight) {
        canvas.height = pixelHeight;
    }
}

function snapTranslation(value, dpr) {
    const normalizedDpr = normalizeDpr(dpr);
    return Math.round(value * normalizedDpr) / normalizedDpr;
}

function drawMarkerPin(targetCtx, marker) {
    const x = marker.x;
    const y = marker.y;
    const color = marker.color || "#2196F3";
    const headY = y - MARKER_HEAD_CENTER_OFFSET_Y;

    targetCtx.fillStyle = color;
    targetCtx.strokeStyle = "#1565C0";
    targetCtx.lineWidth = 1;
    targetCtx.lineJoin = "round";
    targetCtx.beginPath();
    targetCtx.moveTo(x, y);
    targetCtx.lineTo(x - MARKER_TAIL_HALF_WIDTH, y - MARKER_TAIL_TOP_OFFSET_Y);
    targetCtx.lineTo(x + MARKER_TAIL_HALF_WIDTH, y - MARKER_TAIL_TOP_OFFSET_Y);
    targetCtx.closePath();
    targetCtx.fill();
    targetCtx.stroke();

    targetCtx.beginPath();
    targetCtx.arc(x, headY, MARKER_HEAD_RADIUS, 0, Math.PI * 2);
    targetCtx.fill();
    targetCtx.stroke();

    targetCtx.fillStyle = "#ffffff";
    targetCtx.beginPath();
    targetCtx.arc(x, headY, 4.5, 0, Math.PI * 2);
    targetCtx.fill();
}

function drawTileLayer(layer, dpr, now, countMissing) {
    if (!layer || !Array.isArray(layer.tiles) || layer.tiles.length === 0) {
        return { hasMissing: false, hasFading: false };
    }

    const scale = Number.isFinite(layer.scale) ? layer.scale : 1;
    const translateX = Number.isFinite(layer.translate_x) ? layer.translate_x : 0;
    const translateY = Number.isFinite(layer.translate_y) ? layer.translate_y : 0;
    const snappedTranslateX = snapTranslation(translateX, dpr);
    const snappedTranslateY = snapTranslation(translateY, dpr);

    ctx.setTransform(
        dpr * scale,
        0,
        0,
        dpr * scale,
        dpr * snappedTranslateX,
        dpr * snappedTranslateY,
    );

    let hasMissing = false;
    let hasFading = false;
    /** @type {Map<number, Array<{slotIndex: number, x: number, y: number, w: number, h: number, alpha: number}>>} */
    const drawsByPage = new Map();

    for (const tile of layer.tiles) {
        const entry = atlasIndex.get(tile.url);
        if (!entry) {
            if (countMissing) {
                hasMissing = true;
            }
            continue;
        }
        entry.lastUsed = ++useCounter;
        const age = now - entry.loadedAt;
        const alpha = entry.loadedAt > 0 ? Math.min(1, Math.max(0, age / FADE_IN_MS)) : 1;
        if (alpha < 1) {
            hasFading = true;
        }

        let pageDraws = drawsByPage.get(entry.pageIndex);
        if (!pageDraws) {
            pageDraws = [];
            drawsByPage.set(entry.pageIndex, pageDraws);
        }

        pageDraws.push({
            slotIndex: entry.slotIndex,
            x: tile.x,
            y: tile.y,
            w: tile.w,
            h: tile.h,
            alpha,
        });
    }

    for (const [pageIndex, draws] of drawsByPage) {
        const page = atlasPages[pageIndex];
        if (!page) {
            if (countMissing) {
                hasMissing = true;
            }
            continue;
        }

        for (const draw of draws) {
            const [sx, sy] = slotToXY(draw.slotIndex);
            if (draw.alpha < 1) {
                ctx.globalAlpha = draw.alpha;
            } else if (ctx.globalAlpha !== 1) {
                ctx.globalAlpha = 1;
            }
            ctx.drawImage(
                page.canvas,
                sx,
                sy,
                atlasTileSize,
                atlasTileSize,
                draw.x,
                draw.y,
                draw.w,
                draw.h,
            );
        }
    }

    if (ctx.globalAlpha !== 1) {
        ctx.globalAlpha = 1;
    }

    return { hasMissing, hasFading };
}

function readyTileCount(layer) {
    if (!layer || !Array.isArray(layer.tiles) || layer.tiles.length === 0) {
        return 0;
    }
    let ready = 0;
    for (const tile of layer.tiles) {
        if (atlasIndex.has(tile.url)) {
            ready += 1;
        }
    }
    return ready;
}

function bestFallbackLayer(layers) {
    if (!Array.isArray(layers) || layers.length === 0) {
        return null;
    }
    let best = null;
    let bestReady = -1;
    for (const layer of layers) {
        const ready = readyTileCount(layer);
        if (ready > bestReady) {
            best = layer;
            bestReady = ready;
        }
    }
    return best;
}

function drawScene() {
    if (!ctx || !canvas || !currentScene) {
        return;
    }

    const {
        width,
        height,
        dpr,
        primary,
        fallback_layers = [],
        markers = [],
    } = currentScene;

    const desiredUrls = Array.isArray(currentScene.desired_urls) ? currentScene.desired_urls : [];
    if (desiredUrls.length > 0) {
        // Keep retrying failed/missed tiles even when view is static.
        ensureDesiredUrls(desiredUrls);
    }

    ensureCanvasSize(width, height, dpr);
    applyContextQuality(ctx);

    ctx.setTransform(1, 0, 0, 1, 0, 0);
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    const now = nowMs();
    const fallbackLayer = bestFallbackLayer(fallback_layers);
    const fallbackStats = drawTileLayer(fallbackLayer, dpr, now, false);
    const primaryTotal = Array.isArray(primary?.tiles) ? primary.tiles.length : 0;
    const primaryReady = readyTileCount(primary);
    const primaryReadyRatio = primaryTotal > 0 ? (primaryReady / primaryTotal) : 0;
    const fallbackTotal = Array.isArray(fallbackLayer?.tiles) ? fallbackLayer.tiles.length : 0;
    const fallbackReady = readyTileCount(fallbackLayer);
    const fallbackReadyRatio = fallbackTotal > 0 ? (fallbackReady / fallbackTotal) : 0;
    const shouldDeferPrimary = !!fallbackLayer &&
        fallbackReadyRatio >= FALLBACK_REUSE_READY_RATIO &&
        primaryReadyRatio < PRIMARY_REPLACE_READY_RATIO;
    const shouldDrawPrimary = !shouldDeferPrimary;
    const primaryStats = shouldDrawPrimary
        ? drawTileLayer(primary, dpr, now, true)
        : { hasMissing: primaryReady < primaryTotal, hasFading: false };
    const hasMissing = primaryStats.hasMissing;
    const hasFading = primaryStats.hasFading || fallbackStats.hasFading;

    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    for (const marker of markers) {
        drawMarkerPin(ctx, marker);
    }

    if (hasMissing) {
        scheduleDraw(16);
    } else if (hasFading) {
        scheduleDraw(16);
    }
}

function disposeAll() {
    cancelScheduledDraw();
    currentScene = null;
    inFlight.clear();
    failedUntil.clear();
    atlasIndex.clear();
    atlasPages.length = 0;
    canvas = null;
    ctx = null;
}

self.onmessage = (event) => {
    const message = event.data;
    if (!message || typeof message.type !== "string") {
        return;
    }

    if (message.type === "init") {
        canvas = message.canvas ?? null;
        if (!canvas) {
            self.postMessage({ type: "init_failed" });
            return;
        }
        ctx = canvas.getContext("2d", { alpha: true, desynchronized: true });
        if (!ctx) {
            self.postMessage({ type: "init_failed" });
            return;
        }
        applyContextQuality(ctx);
        self.postMessage({ type: "ready" });
        drawScene();
        return;
    }

    if (message.type === "scene") {
        resetAtlasLayout(message.tile_size, message.dpr);
        currentScene = message;
        ensureDesiredUrls(message.desired_urls ?? []);
        drawScene();
        return;
    }

    if (message.type === "dispose") {
        disposeAll();
        self.close();
    }
};

self.__tileWorkerDebug = {
    getStats: () => ({
        pages: atlasPages.length,
        cached: atlasIndex.size,
        inflight: inFlight.size,
        tileSize: atlasTileSize,
        maxCached: maxCachedImages,
    }),
};
