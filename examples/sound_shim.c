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

void loi_audio_cleanup(void) {
    EM_ASM({
        if (window.loiAudioNode) {
            try { window.loiAudioNode.stop(); } catch (e) {}
            window.loiAudioNode = null;
        }
        window.loiAudioPlaying = false;
        if (window.loiAudioContext) {
            window.loiAudioContext.close();
            window.loiAudioContext = null;
        }
    });
}

#else

static Sound loi_synth_sound;
static bool loi_synth_loaded = false;

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

bool loi_samples_playing(void) {
    if (!loi_synth_loaded) {
        return false;
    }
    return IsSoundPlaying(loi_synth_sound);
}

void loi_audio_cleanup(void) {
    if (loi_synth_loaded) {
        UnloadSound(loi_synth_sound);
        loi_synth_loaded = false;
    }
}

#endif
