/* Shim de audio para o exemplo de sintese matematica em Loi.
 *
 * O bindgen pulou Sound/Music/AudioStream porque sao records C inseguros
 * passados por valor. Este shim esconde o estado do player e recebe um slice
 * [f32] de amostras geradas em Loi.
 *
 * No navegador usamos WebAudio direto (AudioContext + AudioBufferSourceNode).
 * O backend WebAudio do miniaudio, usado pelo raylib, exige AudioWorklets com
 * WASM_WORKERS/SharedArrayBuffer, o que precisaria de headers COOP/COEP; o
 * caminho abaixo toca o som sem nada disso e funciona sob o CSP do site.
 *
 * Compile junto com o shim do raylib:
 *   loi build examples/sound_synth.loi -o synth \
 *       --c-source examples/sound_shim.c \
 *       --c-source vendor/raylib/raylib_shim.c \
 *       --cc-flag -Ivendor/raylib/include \
 *       --cc-flag -lraylib --cc-flag -lm
 */
#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "raylib.h"

typedef struct {
    void* data;
    size_t len;
} LoiSlice;

#if defined(__EMSCRIPTEN__)

#include <emscripten.h>

/* raylib's miniaudio backend creates a ScriptProcessorNode as soon as
 * InitAudioDevice() runs. Modern browsers start that callback before
 * Emscripten publishes HEAPF32 on Module, so it throws on every audio tick.
 * The examples below play through WebAudio directly; no raylib audio device is
 * needed (or wanted) in the browser. */
void loi_init_audio_device(void) {
    (void)0;
}

void loi_close_audio_device(void) {
    (void)0;
}

void loi_play_samples(LoiSlice samples, uint32_t sample_rate) {
    if (samples.len == 0 || samples.data == NULL) {
        return;
    }
    EM_ASM({
        var Ctx = window.AudioContext || window.webkitAudioContext;
        if (!Ctx) return;
        if (!window.loiAudioContext) window.loiAudioContext = new Ctx();
        var ctx = window.loiAudioContext;
        if (ctx.state === "suspended") ctx.resume();

        var ptr = $0 >>> 0;
        var len = $1 >>> 0;
        var rate = $2 >>> 0;
        var data = new Float32Array(len);
        for (var i = 0; i < len; ++i) data[i] = HEAPF32[(ptr >>> 2) + i];

        var buffer = ctx.createBuffer(1, len, rate);
        buffer.getChannelData(0).set(data);
        if (window.loiAudioNode) {
            try { window.loiAudioNode.stop(); } catch (e) {}
        }
        var node = ctx.createBufferSource();
        node.buffer = buffer;
        node.connect(ctx.destination);
        window.loiAudioNode = node;
        window.loiAudioPlaying = true;
        node.onended = function () { window.loiAudioPlaying = false; };
        node.start();
    }, (int)(intptr_t)samples.data, (int)samples.len, (int)sample_rate);
}

/* Continuous feed for the audio-visualizer style examples. The caller fills
 * one slice per frame; we schedule the buffers back to back so WebAudio plays
 * them without gaps. No AudioWorklet, SharedArrayBuffer or COOP/COEP needed. */
void loi_audio_feed(LoiSlice samples, uint32_t sample_rate) {
    if (samples.len == 0 || samples.data == NULL) {
        return;
    }
    EM_ASM({
        var Ctx = window.AudioContext || window.webkitAudioContext;
        if (!Ctx) return;
        if (!window.loiAudioContext) window.loiAudioContext = new Ctx();
        var ctx = window.loiAudioContext;
        if (ctx.state === "suspended") ctx.resume();

        var ptr = $0 >>> 0;
        var len = $1 >>> 0;
        var rate = $2 >>> 0;
        if (len === 0 || rate === 0) return;

        var data = new Float32Array(len);
        for (var i = 0; i < len; ++i) data[i] = HEAPF32[(ptr >>> 2) + i];

        var buffer = ctx.createBuffer(1, len, rate);
        buffer.getChannelData(0).set(data);

        if (!window.loiAudioNodes) window.loiAudioNodes = [];
        if (typeof window.loiAudioActive !== "number") window.loiAudioActive = 0;
        if (typeof window.loiNextAudioTime !== "number" ||
            window.loiNextAudioTime < ctx.currentTime) {
            window.loiNextAudioTime = ctx.currentTime + 0.05;
        }

        /* Never queue more than half a second ahead: if the tab throttles we
         * drop new chunks instead of building a large backlog. */
        if (window.loiNextAudioTime > ctx.currentTime + 0.5) return;

        var when = window.loiNextAudioTime;
        var node = ctx.createBufferSource();
        node.buffer = buffer;
        node.connect(ctx.destination);

        window.loiAudioNodes.push(node);
        window.loiAudioActive += 1;
        window.loiAudioPlaying = true;
        node.onended = function () {
            var nodes = window.loiAudioNodes || [];
            var at = nodes.indexOf(node);
            if (at >= 0) nodes.splice(at, 1);
            if (window.loiAudioActive > 0) window.loiAudioActive -= 1;
            window.loiAudioPlaying = window.loiAudioActive > 0;
        };
        node.start(when);
        window.loiNextAudioTime = when + len / rate;
    }, (int)(intptr_t)samples.data, (int)samples.len, (int)sample_rate);
}

void loi_stop_samples(void) {
    EM_ASM({
        if (window.loiAudioNode) {
            try { window.loiAudioNode.stop(); } catch (e) {}
            window.loiAudioNode = null;
        }
        window.loiAudioPlaying = false;
    });
}

bool loi_samples_playing(void) {
    return EM_ASM_INT({ return window.loiAudioPlaying ? 1 : 0; }) != 0;
}

// Browsers start the AudioContext suspended until a user gesture. The wrapper
// creates/resumes it on the first tap or click; this reports when it is safe
// for the Loi program to start playing without waiting for a keyboard key.
bool loi_audio_ready(void) {
    return EM_ASM_INT({
        var ctx = window.loiAudioContext;
        return ctx && ctx.state !== "suspended" && ctx.state !== "closed" ? 1 : 0;
    }) != 0;
}

void loi_audio_cleanup(void) {
    EM_ASM({
        if (window.loiAudioNode) {
            try { window.loiAudioNode.stop(); } catch (e) {}
            window.loiAudioNode = null;
        }
        var nodes = window.loiAudioNodes || [];
        for (var i = 0; i < nodes.length; ++i) {
            try { nodes[i].stop(); } catch (e) {}
        }
        window.loiAudioNodes = [];
        window.loiAudioActive = 0;
        window.loiNextAudioTime = 0;
        window.loiAudioPlaying = false;
        if (window.loiAudioContext) {
            window.loiAudioContext.close();
            window.loiAudioContext = null;
        }
    });
}

#else

void loi_init_audio_device(void) {
    InitAudioDevice();
}

void loi_close_audio_device(void) {
    CloseAudioDevice();
}

static Sound loi_synth_sound;
static bool loi_synth_loaded = false;
static AudioStream loi_visual_stream;
static bool loi_visual_ready = false;
static uint32_t loi_visual_rate = 0;

void loi_play_samples(LoiSlice samples, uint32_t sample_rate) {
    size_t bytes = samples.len * sizeof(float);
    if (bytes == 0 || samples.data == NULL) {
        return;
    }
    if (loi_synth_loaded) {
        StopSound(loi_synth_sound);
        UnloadSound(loi_synth_sound);
        loi_synth_loaded = false;
    }
    Wave wave;
    wave.data = malloc(bytes);
    if (wave.data == NULL) {
        return;
    }
    memcpy(wave.data, samples.data, bytes);
    wave.frameCount = (unsigned int)samples.len;
    wave.sampleRate = sample_rate;
    wave.sampleSize = 32;
    wave.channels = 1;
    loi_synth_sound = LoadSoundFromWave(wave);
    UnloadWave(wave);
    loi_synth_loaded = true;
    PlaySound(loi_synth_sound);
}

void loi_stop_samples(void) {
    if (loi_synth_loaded) {
        StopSound(loi_synth_sound);
    }
}

/* Keep a raylib AudioStream fed from the Loi main loop. This is the native
 * counterpart of the WebAudio buffer queue: continuous procedural audio without
 * exposing raylib's callback type to Loi. */
void loi_audio_feed(LoiSlice samples, uint32_t sample_rate) {
    if (samples.len == 0 || samples.data == NULL) {
        return;
    }
    if (!loi_visual_ready || loi_visual_rate != sample_rate) {
        if (loi_visual_ready) {
            StopAudioStream(loi_visual_stream);
            UnloadAudioStream(loi_visual_stream);
            loi_visual_ready = false;
        }
        loi_visual_stream = LoadAudioStream(sample_rate, 32, 1);
        if (!IsAudioStreamValid(loi_visual_stream)) {
            return;
        }
        loi_visual_rate = sample_rate;
        loi_visual_ready = true;
        PlayAudioStream(loi_visual_stream);
    }
    UpdateAudioStream(loi_visual_stream, samples.data, (int)samples.len);
}

bool loi_samples_playing(void) {
    if (loi_visual_ready) {
        return IsAudioStreamPlaying(loi_visual_stream);
    }
    if (!loi_synth_loaded) {
        return false;
    }
    return IsSoundPlaying(loi_synth_sound);
}

bool loi_audio_ready(void) {
    return IsAudioDeviceReady();
}

void loi_audio_cleanup(void) {
    if (loi_visual_ready) {
        StopAudioStream(loi_visual_stream);
        UnloadAudioStream(loi_visual_stream);
        loi_visual_ready = false;
    }
    if (loi_synth_loaded) {
        UnloadSound(loi_synth_sound);
        loi_synth_loaded = false;
    }
}

#endif
