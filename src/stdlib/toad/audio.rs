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
    /// What is heard, handed on as it is mixed ([`tape`]): each buffer,
    /// interleaved as the device takes it.
    tape: Option<std::sync::mpsc::Sender<Vec<f32>>>,
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
        // taped: this buffer as it will be heard (a copy: the writer is
        // another thread's, and this one never waits)
        if let Some(t) = &self.tape
            && t.send(out.to_vec()).is_err()
        {
            self.tape = None;
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
    /// What is heard being written to a file ([`tape`]).
    tape: Option<Taping>,
    /// The microphone being written to a file ([`mic`]).
    mic: Option<Taping>,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| {
        Mutex::new(State {
            sounds: HashMap::new(),
            next: 1,
            audio: None,
            tape: None,
            mic: None,
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
        tape: None,
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
            sound: s.clone(),
            // `at`: from so many seconds in (a recording played from where it is)
            pos: (n("at", 0.0).max(0.0) as f64 * s.rate as f64).min(s.frames() as f64),
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
        tape: None,
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

// ── taping: what is heard, and the microphone, written as WAV ───────

/// A WAV being written by a thread of its own.
struct Taping {
    path: String,
    /// Dropped (or told) to end it.
    stop: std::sync::mpsc::Sender<()>,
    /// What it wrote, said once it has ended: its frames, rate, width
    /// and the loudest sample (0…1).
    done: std::sync::mpsc::Receiver<(u64, u32, u16, f32)>,
    /// The device it listens to (the microphone's), by name.
    device: String,
}

/// Buffers of samples (interleaved, `channels` wide, at `rate`) written
/// to `path` as a 16-bit WAV until `rx` ends: the frames written and the
/// loudest sample among them.
fn write_wav(
    path: &str,
    rate: u32,
    channels: u16,
    rx: std::sync::mpsc::Receiver<Vec<f32>>,
    stop: std::sync::mpsc::Receiver<()>,
) -> Result<(u64, f32), String> {
    let spec = hound::WavSpec {
        channels,
        sample_rate: rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut w = hound::WavWriter::create(path, spec).map_err(|e| format!("{path}: {e}"))?;
    let mut samples: u64 = 0;
    let mut peak: f32 = 0.0;
    let mut put = |buf: &[f32], w: &mut hound::WavWriter<_>| -> Result<(), String> {
        for x in buf {
            peak = peak.max(x.abs());
            w.write_sample((x.clamp(-1.0, 1.0) * i16::MAX as f32) as i16)
                .map_err(|e| format!("{path}: {e}"))?;
        }
        Ok(())
    };
    loop {
        match rx.recv_timeout(std::time::Duration::from_millis(50)) {
            Ok(buf) => {
                put(&buf, &mut w)?;
                samples += buf.len() as u64;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
        }
        // told to end (or its teller gone): what is waiting first
        if !matches!(stop.try_recv(), Err(std::sync::mpsc::TryRecvError::Empty)) {
            while let Ok(buf) = rx.try_recv() {
                put(&buf, &mut w)?;
                samples += buf.len() as u64;
            }
            break;
        }
    }
    w.finalize().map_err(|e| format!("{path}: {e}"))?;
    Ok((samples / channels.max(1) as u64, peak.min(1.0)))
}

fn taped(t: Taping) -> Value {
    drop(t.stop);
    let (frames, rate, channels, peak) = t
        .done
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap_or((0, 0, 0, 0.0));
    let mut m = ValueMap::default();
    m.insert("path".into(), Value::String(t.path.into()));
    // the loudest sample (0…1): a voice that was never heard is near 0
    m.insert("peak".into(), Value::Float(peak as f64));
    if !t.device.is_empty() {
        m.insert("device".into(), Value::String(t.device.into()));
    }
    m.insert("frames".into(), Value::Integer(frames as i64));
    m.insert("rate".into(), Value::Integer(rate as i64));
    m.insert("channels".into(), Value::Integer(channels as i64));
    m.insert(
        "ms".into(),
        Value::Integer(if rate == 0 { 0 } else { (frames * 1000 / rate as u64) as i64 }),
    );
    Value::Map(Arc::new(m))
}

fn began(rate: u32, channels: u16) -> Value {
    let mut m = ValueMap::default();
    m.insert("rate".into(), Value::Integer(rate as i64));
    m.insert("channels".into(), Value::Integer(channels as i64));
    Value::Map(Arc::new(m))
}

/// `toad.tape(path)`: what is heard (the mixer's output, every bus,
/// after the limiter) written to `path` as a WAV from now, silence too,
/// so its time is the clock's; answers `#{ rate, channels }`.
/// `toad.tape(())` ends it and answers `#{ path, frames, rate, channels,
/// ms }` (`()` when none was on). With no output device (or
/// `TOAD_MUTE=1`) nothing is mixed, so nothing can be taped: an error.
pub fn tape(what: &Value) -> Result<Value, String> {
    match what {
        Value::Unit => Ok(with(|st| {
            let t = st.tape.take();
            if let Some(a) = &st.audio
                && let Ok(mut m) = a.mixer.lock()
            {
                m.tape = None;
            }
            t
        })
        .map_or(Value::Unit, taped)),
        Value::String(path) => with(|st| {
            if st.tape.is_some() {
                return Err("toad.tape: a tape is on already".to_string());
            }
            let a = audio(st);
            let (rate, channels) = match &a.device {
                Ok((_, r, c)) => (*r, *c),
                Err(e) => return Err(format!("toad.tape: nothing is played here to tape ({e})")),
            };
            let (tx, rx) = std::sync::mpsc::channel::<Vec<f32>>();
            let (stop, stopped) = std::sync::mpsc::channel::<()>();
            let (done_tx, done) = std::sync::mpsc::channel();
            let p = path.to_string();
            std::thread::Builder::new()
                .name("toad tape".into())
                .spawn(move || {
                    let (frames, peak) = match write_wav(&p, rate, channels, rx, stopped) {
                        Ok(n) => n,
                        Err(e) => {
                            eprintln!("toad.tape: {e}");
                            (0, 0.0)
                        }
                    };
                    let _ = done_tx.send((frames, rate, channels, peak));
                })
                .map_err(|e| format!("toad.tape: its thread: {e}"))?;
            a.mixer.lock().unwrap_or_else(|e| e.into_inner()).tape = Some(tx);
            st.tape = Some(Taping {
                path: path.to_string(),
                stop,
                done,
                device: String::new(),
            });
            Ok(began(rate, channels))
        }),
        _ => Err("toad.tape: a path begins it, () ends it".to_string()),
    }
}

/// Whether this process may ask for the microphone: an application
/// (`…/X.app/Contents/MacOS/…`) must say why in its Info.plist
/// (`NSMicrophoneUsageDescription`), or macOS ends it at the first ask.
fn mic_allowed() -> Result<(), String> {
    let Ok(exe) = std::env::current_exe() else {
        return Ok(());
    };
    let Some(contents) = exe.parent().and_then(|p| p.parent()) else {
        return Ok(());
    };
    if contents.file_name().is_none_or(|n| n != "Contents") {
        return Ok(());
    }
    match std::fs::read(contents.join("Info.plist")) {
        Ok(b) if String::from_utf8_lossy(&b).contains("NSMicrophoneUsageDescription") => Ok(()),
        _ => Err(
            "toad.mic: this application does not say why it wants the microphone \
             (NSMicrophoneUsageDescription in its Info.plist): not asked for"
                .to_string(),
        ),
    }
}

/// How each audio device of this Mac is connected, by its name
/// (CoreAudio's transport type: cpal does not say it).
#[cfg(target_os = "macos")]
fn transports() -> HashMap<String, &'static str> {
    use std::ffi::c_void;
    #[repr(C)]
    struct Addr {
        selector: u32,
        scope: u32,
        element: u32,
    }
    #[link(name = "CoreAudio", kind = "framework")]
    unsafe extern "C" {
        fn AudioObjectGetPropertyDataSize(
            id: u32,
            addr: *const Addr,
            qsize: u32,
            q: *const c_void,
            size: *mut u32,
        ) -> i32;
        fn AudioObjectGetPropertyData(
            id: u32,
            addr: *const Addr,
            qsize: u32,
            q: *const c_void,
            size: *mut u32,
            data: *mut c_void,
        ) -> i32;
    }
    const fn cc(b: &[u8; 4]) -> u32 {
        u32::from_be_bytes(*b)
    }
    let global = cc(b"glob");
    let mut out = HashMap::new();
    // SAFETY: CoreAudio's documented property calls: the sizes asked for
    // first, the buffers as large as it says, the name a CFString we own.
    unsafe {
        let devs = Addr { selector: cc(b"dev#"), scope: global, element: 0 };
        let mut size: u32 = 0;
        if AudioObjectGetPropertyDataSize(1, &devs, 0, std::ptr::null(), &mut size) != 0 {
            return out;
        }
        let mut ids = vec![0u32; size as usize / 4];
        if AudioObjectGetPropertyData(1, &devs, 0, std::ptr::null(), &mut size, ids.as_mut_ptr().cast()) != 0 {
            return out;
        }
        for id in ids.into_iter().take(size as usize / 4) {
            let mut tran: u32 = 0;
            let mut n = 4u32;
            let a = Addr { selector: cc(b"tran"), scope: global, element: 0 };
            if AudioObjectGetPropertyData(id, &a, 0, std::ptr::null(), &mut n, (&mut tran as *mut u32).cast()) != 0 {
                continue;
            }
            let mut name: *const objc2_core_foundation::CFString = std::ptr::null();
            let mut n = std::mem::size_of::<*const c_void>() as u32;
            let a = Addr { selector: cc(b"lnam"), scope: global, element: 0 };
            if AudioObjectGetPropertyData(id, &a, 0, std::ptr::null(), &mut n, (&mut name as *mut *const objc2_core_foundation::CFString).cast()) != 0 {
                continue;
            }
            let Some(nn) = std::ptr::NonNull::new(name.cast_mut()) else {
                continue;
            };
            let name = objc2_core_foundation::CFRetained::from_raw(nn).to_string();
            let kind = if tran == cc(b"bltn") {
                "built-in"
            } else if tran == cc(b"blue") || tran == cc(b"blea") {
                "bluetooth"
            } else if tran == cc(b"usb ") {
                "usb"
            } else {
                "other"
            };
            out.insert(name, kind);
        }
    }
    out
}

#[cfg(not(target_os = "macos"))]
fn transports() -> HashMap<String, &'static str> {
    HashMap::new()
}

/// A microphone's name.
fn name_of(d: &cpal::Device) -> String {
    use cpal::traits::DeviceTrait;
    d.description()
        .map(|x| x.name().to_string())
        .unwrap_or_else(|_| "microphone".to_string())
}

/// The microphones here: each device, its kind, its name, whether it is
/// the system's.
fn inputs() -> Vec<(cpal::Device, &'static str, String, bool)> {
    use cpal::traits::HostTrait;
    let host = cpal::default_host();
    let sys = host.default_input_device().map(|d| name_of(&d));
    let how = transports();
    host.input_devices()
        .map(|ds| {
            ds.map(|d| {
                let name = name_of(&d);
                let kind = how.get(&name).copied().unwrap_or("other");
                let is_sys = sys.as_deref() == Some(name.as_str());
                (d, kind, name, is_sys)
            })
            .collect()
        })
        .unwrap_or_default()
}

/// The microphone a recording takes when none is named: the system's,
/// unless that is a Bluetooth headset's and the machine has one built
/// in. A headset's own microphone puts the headset in its call mode:
/// everything it plays drops to telephone quality (a hiss, 24 kHz), and
/// a voice recorded through it is poor. The built-in one leaves the
/// headset as it was.
fn pick_input(named: Option<&str>) -> Result<(cpal::Device, &'static str, String), String> {
    let all = inputs();
    if let Some(n) = named {
        return all
            .into_iter()
            .find(|(_, _, name, _)| name == n)
            .map(|(d, k, name, _)| (d, k, name))
            .ok_or_else(|| format!("no microphone named \"{n}\""));
    }
    let sys_bluetooth = all.iter().any(|(_, k, _, is_sys)| *is_sys && *k == "bluetooth");
    let at = if sys_bluetooth {
        all.iter().position(|(_, k, _, _)| *k == "built-in")
    } else {
        None
    }
    .or_else(|| all.iter().position(|(_, _, _, is_sys)| *is_sys))
    .or(if all.is_empty() { None } else { Some(0) })
    .ok_or("no microphone")?;
    let (d, k, name, _) = all.into_iter().nth(at).ok_or("no microphone")?;
    Ok((d, k, name))
}

/// `toad.mics()`: the microphones here, `[#{ name, kind, system, chosen
/// }]` — `kind` `"built-in"`, `"bluetooth"`, `"usb"` or `"other"`,
/// `system` the one the system's settings name, `chosen` the one
/// `toad.mic` takes when none is named (see [`pick_input`]). Asking
/// opens no microphone.
pub fn mics() -> Value {
    let chosen = pick_input(None).map(|(_, _, n)| n).ok();
    Value::List(Arc::new(
        inputs()
            .into_iter()
            .map(|(_, kind, name, is_sys)| {
                let mut m = ValueMap::default();
                m.insert("kind".into(), Value::String(kind.to_string().into()));
                m.insert("system".into(), Value::Boolean(is_sys));
                m.insert("chosen".into(), Value::Boolean(chosen.as_deref() == Some(name.as_str())));
                m.insert("name".into(), Value::String(name.into()));
                Value::Map(Arc::new(m))
            })
            .collect(),
    ))
}

/// `toad.mic(path)`: a microphone written to `path` as a WAV from now;
/// answers `#{ rate, channels, device, kind }`. `toad.mic(#{ path,
/// device })` names which (a name of `toad.mics()`); else the one
/// `toad.mics()` says is `chosen`. The first time, macOS asks the
/// person. `toad.mic(())` ends it and answers `#{ path, frames, rate,
/// channels, ms, peak, device }` (`()` when none was on): `peak` the
/// loudest sample, near 0 when nothing was heard.
pub fn mic(what: &Value) -> Result<Value, String> {
    let (path, named): (&str, Option<String>) = match what {
        Value::Unit => return Ok(with(|st| st.mic.take()).map_or(Value::Unit, taped)),
        Value::String(p) => (p.as_ref(), None),
        rec @ (Value::Map(_) | Value::Struct { .. }) => match rec.record_fields().and_then(|m| m.get("path")) {
            Some(Value::String(p)) => (
                p.as_ref(),
                match rec.record_fields().and_then(|m| m.get("device")) {
                    Some(Value::String(d)) => Some(d.to_string()),
                    _ => None,
                },
            ),
            _ => return Err("toad.mic: `path` (where to write it) is needed".to_string()),
        },
        _ => return Err("toad.mic: a path begins it, () ends it".to_string()),
    };
    {
        {
            if with(|st| st.mic.is_some()) {
                return Err("toad.mic: the microphone is being recorded already".to_string());
            }
            mic_allowed()?;
            let (stop, stopped) = std::sync::mpsc::channel::<()>();
            let (done_tx, done) = std::sync::mpsc::channel();
            let (up_tx, up) =
                std::sync::mpsc::channel::<Result<(u32, u16, &'static str, String), String>>();
            let p = path.to_string();
            std::thread::Builder::new()
                .name("toad mic".into())
                .spawn(move || {
                    use cpal::traits::{DeviceTrait, StreamTrait};
                    let (tx, rx) = std::sync::mpsc::channel::<Vec<f32>>();
                    let made = (|| -> Result<(cpal::Stream, u32, u16, &'static str, String), String> {
                        let (device, kind, dname) = pick_input(named.as_deref())?;
                        let config = device
                            .default_input_config()
                            .map_err(|e| format!("the microphone: {e}"))?;
                        let format = config.sample_format();
                        let cfg: cpal::StreamConfig = config.into();
                        let (rate, channels) = (cfg.sample_rate, cfg.channels);
                        let err = |e: cpal::Error| eprintln!("toad mic: {e}");
                        let stream = match format {
                            cpal::SampleFormat::F32 => device.build_input_stream(
                                cfg,
                                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                                    let _ = tx.send(data.to_vec());
                                },
                                err,
                                None,
                            ),
                            cpal::SampleFormat::I16 => device.build_input_stream(
                                cfg,
                                move |data: &[i16], _: &cpal::InputCallbackInfo| {
                                    let _ = tx.send(
                                        data.iter().map(|x| *x as f32 / i16::MAX as f32).collect(),
                                    );
                                },
                                err,
                                None,
                            ),
                            other => {
                                return Err(format!(
                                    "the microphone's samples are {other}, not f32 or i16"
                                ));
                            }
                        }
                        .map_err(|e| format!("the microphone: {e}"))?;
                        stream.play().map_err(|e| format!("the microphone: {e}"))?;
                        Ok((stream, rate, channels, kind, dname))
                    })();
                    match made {
                        Err(e) => {
                            let _ = up_tx.send(Err(e));
                        }
                        Ok((stream, rate, channels, kind, dname)) => {
                            let _ = up_tx.send(Ok((rate, channels, kind, dname)));
                            // the stream listens while this thread holds it
                            let (frames, peak) = match write_wav(&p, rate, channels, rx, stopped) {
                                Ok(n) => n,
                                Err(e) => {
                                    eprintln!("toad.mic: {e}");
                                    (0, 0.0)
                                }
                            };
                            drop(stream);
                            let _ = done_tx.send((frames, rate, channels, peak));
                        }
                    }
                })
                .map_err(|e| format!("toad.mic: its thread: {e}"))?;
            let (rate, channels, kind, dname) = up
                .recv_timeout(std::time::Duration::from_secs(10))
                .unwrap_or_else(|_| Err("the microphone did not open".into()))
                .map_err(|e| format!("toad.mic: {e}"))?;
            with(|st| {
                st.mic = Some(Taping {
                    path: path.to_string(),
                    stop,
                    done,
                    device: dname.clone(),
                })
            });
            let mut m = ValueMap::default();
            m.insert("rate".into(), Value::Integer(rate as i64));
            m.insert("channels".into(), Value::Integer(channels as i64));
            m.insert("device".into(), Value::String(dname.into()));
            m.insert("kind".into(), Value::String(kind.to_string().into()));
            Ok(Value::Map(Arc::new(m)))
        }
    }
}
