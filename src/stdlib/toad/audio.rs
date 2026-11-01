//! Sound: a small mixer for games. Sounds are made in code (a synth of
//! waves, slides, envelopes and layers: an arcade game needs no files) or
//! read from WAV and Ogg Vorbis files; voices play them with a volume, a
//! pan, a pitch and a loop, on the effects bus or the music bus.
//!
//! - **The output** is the system's (cpal: CoreAudio on the Mac), opened
//!   the first time a sound plays, on a thread of its own that keeps the
//!   stream. The mixer runs in the device's callback and never waits on
//!   the program: when the program holds the mixer for a moment the
//!   callback plays on with what it had.
//! - **No clicks.** A voice's volume moves toward where it is set over a
//!   few milliseconds (or the fade asked for), a stop is a fade to nothing,
//!   and the sum goes through a soft limiter.
//! - **Silent when it must be.** With no output device, or `TOAD_MUTE=1`
//!   (tests, servers), everything works and nothing is heard.
//! - **Sounds are the same every time:** the synth's noise has a seed.

use super::{map_of, num};
use crate::ast::{Value, ValueMap};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

/// The synth's rate; a sound is played at the device's by its step.
const SYNTH_RATE: u32 = 48_000;
/// Voices at once; past it the oldest effect gives way.
const VOICES: usize = 64;
/// A sound at most (seconds), made or read.
const LONGEST: f32 = 600.0;

/// Samples, interleaved by channel (1 or 2), at `rate`.
pub struct Sound {
    data: Vec<f32>,
    channels: usize,
    rate: u32,
}

impl Sound {
    fn frames(&self) -> usize {
        self.data.len() / self.channels
    }
    fn frame(&self, i: usize) -> (f32, f32) {
        if self.channels == 1 {
            let x = self.data[i];
            (x, x)
        } else {
            (self.data[i * 2], self.data[i * 2 + 1])
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Bus {
    Fx,
    Music,
}

struct Voice {
    id: i64,
    sound: Arc<Sound>,
    pos: f64,
    step: f64,
    pitch: f64,
    gain: f32,
    target: f32,
    /// How far the gain moves toward its target each frame.
    glide: f32,
    pan: f32,
    looped: bool,
    stopping: bool,
    bus: Bus,
}

struct Mixer {
    rate: u32,
    voices: Vec<Voice>,
    master: f32,
    fx: f32,
    music: f32,
}

impl Mixer {
    fn glide_for(&self, secs: f32) -> f32 {
        // the gain covers the way in about `secs`
        let frames = (secs.max(0.004) * self.rate as f32).max(1.0);
        1.0 - (-5.0 / frames).exp()
    }

    /// Fill `out` (interleaved, `channels` wide).
    fn mix(&mut self, out: &mut [f32], channels: usize) {
        for x in out.iter_mut() {
            *x = 0.0;
        }
        let frames = out.len() / channels.max(1);
        let (fx, music, master) = (self.fx, self.music, self.master);
        for v in self.voices.iter_mut() {
            let n = v.sound.frames();
            if n == 0 {
                v.stopping = true;
                v.gain = 0.0;
                continue;
            }
            let bus = if v.bus == Bus::Music { music } else { fx };
            // equal-power pan
            let a = (v.pan.clamp(-1.0, 1.0) + 1.0) * std::f32::consts::FRAC_PI_4;
            let (pl, pr) = (a.cos(), a.sin());
            for f in 0..frames {
                v.gain += (v.target - v.gain) * v.glide;
                let i = v.pos as usize;
                if i >= n {
                    if v.looped {
                        v.pos -= n as f64;
                        continue;
                    }
                    v.stopping = true;
                    v.gain = 0.0;
                    break;
                }
                // linear between this frame and the next
                let t = (v.pos - i as f64) as f32;
                let (l0, r0) = v.sound.frame(i);
                let (l1, r1) = v.sound.frame(if i + 1 < n {
                    i + 1
                } else if v.looped {
                    0
                } else {
                    i
                });
                let l = l0 + (l1 - l0) * t;
                let r = r0 + (r1 - r0) * t;
                let g = v.gain * bus;
                let at = f * channels;
                if channels == 1 {
                    out[at] += (l + r) * 0.5 * g;
                } else {
                    out[at] += l * g * pl * std::f32::consts::SQRT_2;
                    out[at + 1] += r * g * pr * std::f32::consts::SQRT_2;
                }
                v.pos += v.step * v.pitch;
            }
        }
        // a stopped voice goes once it has faded
        self.voices.retain(|v| !(v.stopping && v.gain.abs() < 1e-4));
        for x in out.iter_mut() {
            // a soft limit: quiet sums pass, loud ones round off
            let y = *x * master;
            *x = if y.abs() < 0.6 {
                y
            } else {
                y.signum() * (0.6 + 0.4 * ((y.abs() - 0.6) / 0.4).tanh())
            };
        }
    }
}

struct Audio {
    mixer: Arc<Mutex<Mixer>>,
    /// The output's name, rate and width; `None` when silent (and why).
    device: Result<(String, u32, u16), String>,
}

struct State {
    sounds: HashMap<i64, Arc<Sound>>,
    next: i64,
    audio: Option<Audio>,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| {
        Mutex::new(State {
            sounds: HashMap::new(),
            next: 1,
            audio: None,
        })
    })
}

fn with<T>(f: impl FnOnce(&mut State) -> T) -> T {
    let mut g = state().lock().unwrap_or_else(|e| e.into_inner());
    f(&mut g)
}

/// Open the output: a thread that makes the stream and keeps it.
fn open() -> Audio {
    let mixer = Arc::new(Mutex::new(Mixer {
        rate: SYNTH_RATE,
        voices: Vec::new(),
        master: 1.0,
        fx: 1.0,
        music: 1.0,
    }));
    if std::env::var("TOAD_MUTE").is_ok_and(|v| v == "1") {
        return Audio {
            mixer,
            device: Err("muted (TOAD_MUTE=1)".into()),
        };
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let m = mixer.clone();
    let spawned = std::thread::Builder::new()
        .name("toad audio".into())
        .spawn(move || {
            match start(m) {
                Ok((stream, info)) => {
                    let _ = tx.send(Ok(info));
                    // the stream plays while this thread holds it
                    let _keep = stream;
                    loop {
                        std::thread::park();
                    }
                }
                Err(e) => {
                    let _ = tx.send(Err(e));
                }
            }
        });
    let device = match spawned {
        Ok(_) => rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap_or_else(|_| Err("the audio output did not open".into())),
        Err(e) => Err(format!("the audio thread: {e}")),
    };
    if let Ok((_, rate, _)) = &device
        && let Ok(mut g) = mixer.lock()
    {
        g.rate = *rate;
    }
    Audio { mixer, device }
}

fn start(mixer: Arc<Mutex<Mixer>>) -> Result<(cpal::Stream, (String, u32, u16)), String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("no audio output device")?;
    let name = device
        .id()
        .map(|d| d.to_string())
        .unwrap_or_else(|_| "output".into());
    let config = device
        .default_output_config()
        .map_err(|e| format!("the audio output: {e}"))?;
    let format = config.sample_format();
    let cfg: cpal::StreamConfig = config.into();
    let (rate, channels) = (cfg.sample_rate, cfg.channels);
    let err = |e: cpal::Error| eprintln!("toad audio: {e}");
    let stream = match format {
        cpal::SampleFormat::F32 => device.build_output_stream(
            cfg,
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                // never wait on the program: when it holds the mixer,
                // play silence for this buffer
                match mixer.try_lock() {
                    Ok(mut m) => m.mix(data, channels as usize),
                    Err(_) => data.fill(0.0),
                }
            },
            err,
            None,
        ),
        cpal::SampleFormat::I16 => {
            let mut buf: Vec<f32> = Vec::new();
            device.build_output_stream(
                cfg,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    buf.resize(data.len(), 0.0);
                    match mixer.try_lock() {
                        Ok(mut m) => m.mix(&mut buf, channels as usize),
                        Err(_) => buf.fill(0.0),
                    }
                    for (o, x) in data.iter_mut().zip(buf.iter()) {
                        *o = (x.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                    }
                },
                err,
                None,
            )
        }
        other => {
            return Err(format!(
                "the audio output's samples are {other}, not f32 or i16"
            ));
        }
    }
    .map_err(|e| format!("the audio output: {e}"))?;
    stream
        .play()
        .map_err(|e| format!("the audio output: {e}"))?;
    Ok((stream, (name, rate, channels)))
}

fn audio(st: &mut State) -> &Audio {
    st.audio.get_or_insert_with(open)
}

// ── making sounds ───────────────────────────────────────────────────────

fn splitmix(s: &mut u64) -> f32 {
    *s = s.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *s;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    (z >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0
}

/// One synth voice into `out` (mono, at the synth's rate), from `at`.
fn synth_into(m: &ValueMap, out: &mut Vec<f32>, at: usize) -> Result<(), String> {
    let n = |k: &str, d: f32| m.get(k).and_then(num).unwrap_or(d);
    let wave = match m.get("wave") {
        Some(Value::String(w)) => w.to_string(),
        None | Some(Value::Unit) => "sine".into(),
        Some(_) => return Err("toad.sound: `wave` is a name".into()),
    };
    if !["sine", "square", "saw", "triangle", "noise"].contains(&wave.as_str()) {
        return Err(format!(
            "toad.sound: a wave is sine, square, saw, triangle or noise, not {wave}"
        ));
    }
    let attack = n("attack", 0.005).max(0.0);
    let hold = n("hold", 0.0).max(0.0);
    let decay = n("decay", 0.2).max(0.0);
    let total = (attack + hold + decay).min(LONGEST);
    let frames = (total * SYNTH_RATE as f32) as usize;
    let f0 = n("freq", 440.0).max(1.0);
    let f1 = n("to", f0).max(1.0);
    let duty = n("duty", 0.5).clamp(0.05, 0.95);
    let vib = n("vibrato", 0.0);
    let vib_rate = n("vibrato_rate", 6.0);
    let volume = n("volume", 0.5);
    let cutoff0 = n("cutoff", 0.0);
    let cutoff1 = n("cutoff_to", cutoff0);
    let mut rng = 0x5EED_u64 ^ (n("seed", 0.0) as u64).wrapping_mul(0x2545_F491_4F6C_DD1D);
    let need = at + frames;
    if out.len() < need {
        out.resize(need, 0.0);
    }
    let sr = SYNTH_RATE as f32;
    let (mut phase, mut held, mut lp) = (0.0f32, 0.0f32, 0.0f32);
    for i in 0..frames {
        let t = i as f32 / sr;
        let k = if total > 0.0 { t / total } else { 0.0 };
        // a slide is exponential: it sounds even across octaves
        let f = f0 * (f1 / f0).powf(k) * (1.0 + vib * (t * vib_rate * std::f32::consts::TAU).sin());
        let before = phase;
        phase = (phase + f / sr).fract();
        let x = match wave.as_str() {
            "sine" => (phase * std::f32::consts::TAU).sin(),
            "square" => {
                if phase < duty {
                    1.0
                } else {
                    -1.0
                }
            }
            "saw" => phase * 2.0 - 1.0,
            "triangle" => 1.0 - 4.0 * (phase - 0.5).abs(),
            _ => {
                // noise held for each cycle of `freq`: lower is grittier
                if phase < before {
                    held = splitmix(&mut rng);
                }
                held
            }
        };
        let env = if t < attack {
            t / attack.max(1e-6)
        } else if t < attack + hold {
            1.0
        } else {
            let d = (t - attack - hold) / decay.max(1e-6);
            (1.0 - d).max(0.0).powi(2)
        };
        let mut y = x * env * volume;
        if cutoff0 > 0.0 {
            let c = cutoff0 + (cutoff1 - cutoff0) * k;
            let a = 1.0 - (-std::f32::consts::TAU * c.max(10.0) / sr).exp();
            lp += (y - lp) * a;
            y = lp;
        }
        out[at + i] += y;
    }
    Ok(())
}

/// A sound from a spec: `file` (a WAV or Ogg Vorbis path), or the synth's
/// keys, or `layers` (a list of synth specs, each from its `delay`).
fn make(spec: &Value) -> Result<Sound, String> {
    let m = map_of(spec, "a sound")?;
    if let Some(Value::String(path)) = m.get("file") {
        return read(path);
    }
    let mut out = Vec::new();
    match m.get("layers") {
        Some(Value::List(ls)) => {
            for l in ls.iter() {
                let lm = map_of(l, "a sound's layer")?;
                let at = (lm
                    .get("delay")
                    .and_then(num)
                    .unwrap_or(0.0)
                    .clamp(0.0, LONGEST)
                    * SYNTH_RATE as f32) as usize;
                synth_into(lm, &mut out, at)?;
            }
        }
        _ => synth_into(m, &mut out, 0)?,
    }
    Ok(Sound {
        data: out,
        channels: 1,
        rate: SYNTH_RATE,
    })
}

fn read(path: &str) -> Result<Sound, String> {
    let lower = path.to_lowercase();
    if lower.ends_with(".wav") {
        let mut r = hound::WavReader::open(path).map_err(|e| format!("toad.sound: {path}: {e}"))?;
        let spec = r.spec();
        let channels = spec.channels as usize;
        let data: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => r.samples::<f32>().collect::<Result<_, _>>(),
            hound::SampleFormat::Int => {
                let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
                r.samples::<i32>()
                    .map(|s| s.map(|x| x as f32 / scale))
                    .collect::<Result<_, _>>()
            }
        }
        .map_err(|e| format!("toad.sound: {path}: {e}"))?;
        return fit(data, channels, spec.sample_rate, path);
    }
    if lower.ends_with(".ogg") {
        let f = std::fs::File::open(path).map_err(|e| format!("toad.sound: {path}: {e}"))?;
        let mut r = lewton::inside_ogg::OggStreamReader::new(std::io::BufReader::new(f))
            .map_err(|e| format!("toad.sound: {path}: {e}"))?;
        let channels = r.ident_hdr.audio_channels as usize;
        let rate = r.ident_hdr.audio_sample_rate;
        let mut data = Vec::new();
        while let Some(p) = r
            .read_dec_packet_itl()
            .map_err(|e| format!("toad.sound: {path}: {e}"))?
        {
            data.extend(p.into_iter().map(|x| x as f32 / 32768.0));
            if data.len() > (LONGEST * rate as f32) as usize * channels {
                return Err(format!("toad.sound: {path} is longer than {LONGEST} s"));
            }
        }
        return fit(data, channels, rate, path);
    }
    Err(format!("toad.sound: {path}: a sound file is .wav or .ogg"))
}

/// Mono and stereo kept; more channels folded to their first two.
fn fit(data: Vec<f32>, channels: usize, rate: u32, path: &str) -> Result<Sound, String> {
    match channels {
        1 | 2 => Ok(Sound {
            data,
            channels,
            rate,
        }),
        0 => Err(format!("toad.sound: {path} has no channels")),
        c => Ok(Sound {
            data: data.chunks(c).flat_map(|f| [f[0], f[1]]).collect(),
            channels: 2,
            rate,
        }),
    }
}

// ── the module's functions ──────────────────────────────────────────────

fn info(s: &Sound) -> Value {
    let mut m = ValueMap::default();
    m.insert(
        "seconds".into(),
        Value::Float(s.frames() as f64 / s.rate as f64),
    );
    m.insert("channels".into(), Value::Integer(s.channels as i64));
    m.insert("rate".into(), Value::Integer(s.rate as i64));
    m.insert(
        "peak".into(),
        Value::Float(s.data.iter().fold(0.0f32, |a, x| a.max(x.abs())) as f64),
    );
    Value::Map(Arc::new(m))
}

/// `toad.sound(spec)`: its number.
pub fn sound(spec: &Value) -> Result<Value, String> {
    let s = Arc::new(make(spec)?);
    Ok(with(|st| {
        let id = st.next;
        st.next += 1;
        st.sounds.insert(id, s);
        Value::Integer(id)
    }))
}

/// `toad.sound_info(sound)`: `#{ seconds, channels, rate, peak }`.
pub fn sound_info(id: i64) -> Result<Value, String> {
    with(|st| {
        st.sounds
            .get(&id)
            .map(|s| info(s))
            .ok_or_else(|| format!("toad: no sound {id}"))
    })
}

/// `toad.play(sound, opts)`: a voice's number. `opts`: `volume`, `pan`
/// (-1 left to 1 right), `pitch` (a rate: 2 an octave up), `loop`,
/// `music` (the music bus), `fade` (seconds in).
pub fn play(id: i64, opts: &Value) -> Result<Value, String> {
    let empty = ValueMap::default();
    let m = match opts {
        Value::Unit => &empty,
        o => map_of(o, "the play's options")?,
    };
    let n = |k: &str, d: f32| m.get(k).and_then(num).unwrap_or(d);
    let flag = |k: &str| matches!(m.get(k), Some(Value::Boolean(true)));
    with(|st| {
        let s = st
            .sounds
            .get(&id)
            .cloned()
            .ok_or_else(|| format!("toad: no sound {id}"))?;
        let vid = st.next;
        st.next += 1;
        let a = audio(st);
        let mut mx = a.mixer.lock().unwrap_or_else(|e| e.into_inner());
        let fade = n("fade", 0.0);
        let glide = mx.glide_for(if fade > 0.0 { fade } else { 0.004 });
        let step = s.rate as f64 / mx.rate as f64;
        if mx.voices.len() >= VOICES {
            // the oldest effect gives way (music keeps playing)
            if let Some(i) = mx.voices.iter().position(|v| v.bus == Bus::Fx) {
                mx.voices.remove(i);
            }
        }
        let start = if fade > 0.0 { 0.0 } else { n("volume", 1.0) };
        mx.voices.push(Voice {
            id: vid,
            sound: s,
            pos: 0.0,
            step,
            pitch: n("pitch", 1.0).clamp(0.01, 16.0) as f64,
            gain: start,
            target: n("volume", 1.0).max(0.0),
            glide,
            pan: n("pan", 0.0),
            looped: flag("loop"),
            stopping: false,
            bus: if flag("music") { Bus::Music } else { Bus::Fx },
        });
        Ok(Value::Integer(vid))
    })
}

/// `toad.stop(voice, fade)`: fade it out (seconds) and let it go.
pub fn stop(vid: i64, fade: f32) {
    with(|st| {
        if let Some(a) = &st.audio {
            let mut mx = a.mixer.lock().unwrap_or_else(|e| e.into_inner());
            let glide = mx.glide_for(fade);
            for v in mx.voices.iter_mut().filter(|v| v.id == vid) {
                v.target = 0.0;
                v.glide = glide;
                v.stopping = true;
            }
        }
    })
}

/// `toad.tune(voice, opts)`: `volume`, `pan`, `pitch`, `fade` (seconds
/// for the volume to get there).
pub fn tune(vid: i64, opts: &Value) -> Result<(), String> {
    let m = map_of(opts, "the tuning")?;
    with(|st| {
        if let Some(a) = &st.audio {
            let mut mx = a.mixer.lock().unwrap_or_else(|e| e.into_inner());
            let glide = m.get("fade").and_then(num).map(|f| mx.glide_for(f));
            for v in mx.voices.iter_mut().filter(|v| v.id == vid && !v.stopping) {
                if let Some(x) = m.get("volume").and_then(num) {
                    v.target = x.max(0.0);
                }
                if let Some(g) = glide {
                    v.glide = g;
                }
                if let Some(x) = m.get("pan").and_then(num) {
                    v.pan = x;
                }
                if let Some(x) = m.get("pitch").and_then(num) {
                    v.pitch = x.clamp(0.01, 16.0) as f64;
                }
            }
        }
    });
    Ok(())
}

/// `toad.volume(bus, v)`: `"master"`, `"fx"` or `"music"`.
pub fn volume(bus: &str, v: f32) -> Result<(), String> {
    with(|st| {
        let a = audio(st);
        let mut mx = a.mixer.lock().unwrap_or_else(|e| e.into_inner());
        match bus {
            "master" => mx.master = v.max(0.0),
            "fx" => mx.fx = v.max(0.0),
            "music" => mx.music = v.max(0.0),
            other => {
                return Err(format!(
                    "toad.volume: a bus is master, fx or music, not {other}"
                ));
            }
        }
        Ok(())
    })
}

/// `toad.audio()`: `Ok(#{ device, rate, channels, voices })`, or
/// `Err(why it is silent)`.
pub fn status() -> Value {
    with(|st| {
        let a = audio(st);
        let voices = a.mixer.lock().map(|m| m.voices.len()).unwrap_or(0);
        match &a.device {
            Ok((name, rate, ch)) => {
                let mut m = ValueMap::default();
                m.insert("device".into(), Value::String(Arc::new(name.clone())));
                m.insert("rate".into(), Value::Integer(*rate as i64));
                m.insert("channels".into(), Value::Integer(*ch as i64));
                m.insert("voices".into(), Value::Integer(voices as i64));
                super::ok(Value::Map(Arc::new(m)))
            }
            Err(e) => super::err(e.clone()),
        }
    })
}

/// `toad.listen(sound, seconds)`: a sound mixed offline as the output
/// would hear it (stereo, at the synth's rate), for tests: `#{ peak, rms,
/// voices_left }`.
pub fn listen(id: i64, seconds: f32) -> Result<Value, String> {
    let s = with(|st| st.sounds.get(&id).cloned()).ok_or_else(|| format!("toad: no sound {id}"))?;
    let mut mx = Mixer {
        rate: SYNTH_RATE,
        voices: Vec::new(),
        master: 1.0,
        fx: 1.0,
        music: 1.0,
    };
    mx.voices.push(Voice {
        id: 0,
        step: s.rate as f64 / SYNTH_RATE as f64,
        sound: s,
        pos: 0.0,
        pitch: 1.0,
        gain: 1.0,
        target: 1.0,
        glide: 1.0,
        pan: 0.0,
        looped: false,
        stopping: false,
        bus: Bus::Fx,
    });
    let mut out = vec![0.0f32; ((seconds.clamp(0.0, LONGEST) * SYNTH_RATE as f32) as usize) * 2];
    mx.mix(&mut out, 2);
    let peak = out.iter().fold(0.0f32, |a, x| a.max(x.abs()));
    let rms = (out.iter().map(|x| x * x).sum::<f32>() / out.len().max(1) as f32).sqrt();
    let mut m = ValueMap::default();
    m.insert("peak".into(), Value::Float(peak as f64));
    m.insert("rms".into(), Value::Float(rms as f64));
    m.insert("voices_left".into(), Value::Integer(mx.voices.len() as i64));
    Ok(Value::Map(Arc::new(m)))
}
