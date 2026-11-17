/* Shim de audio para o exemplo de sintese matematica em Loi.
 *
 * O bindgen pulou Sound/Music/AudioStream porque sao records C inseguros
 * passados por valor. Este shim esconde o Sound dentro de um estado estatico
 * e recebe um slice [f32] de amostras geradas em Loi.
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
