// Downsamples microphone input to 8 kHz mono and encodes ITU-T G.711 A-law (PCMA)
// in 20 ms (160-byte) chunks for pytapo's Talk backchannel.

const TARGET_RATE = 8000;
const CHUNK_SIZE = 160;

function encodeAlaw(sample) {
  let sign = 0;
  if (sample < 0) {
    sign = 0x80;
    sample = -sample - 1;
  }
  if (sample > 32767) sample = 32767;
  let exponent = 7;
  for (let mask = 0x4000; (sample & mask) === 0 && exponent > 0; exponent--, mask >>= 1) {}
  const mantissa = (sample >> (exponent === 0 ? 4 : exponent + 3)) & 0x0f;
  return (sign | (exponent << 4) | mantissa) ^ 0xd5;
}

class TalkProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this._ratio = sampleRate / TARGET_RATE;
    this._acc = 0;
    this._sum = 0;
    this._count = 0;
    this._out = new Uint8Array(CHUNK_SIZE);
    this._pos = 0;
  }

  process(inputs) {
    const channel = inputs[0]?.[0];
    if (!channel) return true;
    for (let i = 0; i < channel.length; i++) {
      this._sum += channel[i];
      this._count++;
      this._acc += 1;
      if (this._acc >= this._ratio) {
        this._acc -= this._ratio;
        const avg = Math.max(-1, Math.min(1, this._sum / this._count));
        const pcm = avg < 0 ? Math.round(avg * 32768) : Math.round(avg * 32767);
        this._out[this._pos++] = encodeAlaw(pcm);
        this._sum = 0;
        this._count = 0;
        if (this._pos === CHUNK_SIZE) {
          const buf = this._out;
          this.port.postMessage(buf, [buf.buffer]);
          this._out = new Uint8Array(CHUNK_SIZE);
          this._pos = 0;
        }
      }
    }
    return true;
  }
}

registerProcessor("tapfeed-talk", TalkProcessor);
