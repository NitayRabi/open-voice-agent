// A dependency-free WebGL2 voice visualizer. The entire orb is rendered in one
// fragment shader so it remains cheap enough for the always-on-top Tauri view.
const VERTEX = `#version 300 es
in vec2 position;
void main() { gl_Position = vec4(position, 0.0, 1.0); }
`;

const FRAGMENT = `#version 300 es
precision highp float;
out vec4 fragColor;
uniform vec2 uResolution;
uniform float uTime;
uniform float uInput;
uniform float uOutput;
uniform float uState;
uniform float uWave[128];

#define PI 3.14159265359

float hash(vec2 p) {
  p = fract(p * vec2(123.34, 456.21));
  p += dot(p, p + 45.32);
  return fract(p.x * p.y);
}

float noise(vec2 p) {
  vec2 i = floor(p), f = fract(p);
  f = f * f * (3.0 - 2.0 * f);
  return mix(mix(hash(i), hash(i + vec2(1,0)), f.x),
             mix(hash(i + vec2(0,1)), hash(i + vec2(1)), f.x), f.y);
}

float fbm(vec2 p) {
  float v = 0.0, a = .52;
  mat2 r = mat2(.80, -.60, .60, .80);
  for (int i = 0; i < 5; i++) { v += a * noise(p); p = r * p * 2.03 + 1.7; a *= .5; }
  return v;
}

float waveSample(int index) {
  return uWave[(index + 128) % 128];
}

float rawWaveform(float angle) {
  float at = fract(angle / (2.0 * PI) + .5) * 128.0;
  int i = int(floor(at));
  float f = fract(at);
  float a = waveSample(i - 1), b = waveSample(i);
  float c = waveSample(i + 1), d = waveSample(i + 2);
  // Catmull-Rom interpolation produces a continuous curved circumference
  // instead of straight segments between analyser samples.
  return b + .5 * f * (c-a + f * (2.0*a-5.0*b+4.0*c-d + f * (3.0*(b-c)+d-a)));
}

float waveform(float angle) {
  // A small circular blur keeps individual PCM samples from becoming spikes,
  // while retaining enough detail for the edge to read as a waveform.
  float stepAngle = 2.0 * PI / 128.0;
  return rawWaveform(angle) * .42
    + (rawWaveform(angle-stepAngle) + rawWaveform(angle+stepAngle)) * .22
    + (rawWaveform(angle-stepAngle*2.0) + rawWaveform(angle+stepAngle*2.0)) * .07;
}

vec3 palette(float state, float field) {
  vec3 idle = mix(vec3(.055,.075,.12), vec3(.22,.34,.52), field);
  vec3 listen = mix(vec3(.015,.16,.28), vec3(.05,.82,1.0), field);
  vec3 user = mix(vec3(.02,.20,.18), vec3(.15,1.0,.60), field);
  vec3 think = mix(vec3(.20,.07,.27), vec3(1.0,.36,.76), field);
  vec3 speak = mix(vec3(.08,.04,.30), vec3(.52,.30,1.0), field);
  vec3 error = mix(vec3(.24,.015,.025), vec3(1.0,.14,.08), field);
  if (state < .5) return idle;
  if (state < 1.5) return mix(think, listen, .25);
  if (state < 2.5) return listen;
  if (state < 3.5) return user;
  if (state < 5.5) return think;
  if (state < 6.5) return speak;
  return error;
}

void main() {
  vec2 uv = (gl_FragCoord.xy * 2.0 - uResolution) / min(uResolution.x, uResolution.y);
  float t = uTime;
  float inputKick = uInput * uInput;
  float outputKick = uOutput;
  float activity = max(inputKick, outputKick);

  // Both directions share one stable body. The real PCM waveform is wrapped
  // around its circumference, turning the silhouette into an oscilloscope.
  float ang = atan(uv.y, uv.x);
  float radius = length(uv);
  float audioWave = waveform(ang);
  float wobble = audioWave * .072;
  vec2 p = uv / (.73 + wobble);
  float d = length(p);

  vec2 q = p;
  float warp = fbm(q * 2.2 + vec2(t*.12, -t*.09));
  q += .34 * vec2(cos(warp*6.0+t*.3), sin(warp*5.0-t*.25));
  float storm = fbm(q * (3.4 + activity * 1.6) + warp * 1.8);
  float veins = pow(abs(sin((storm + ang*.12 - t*.08) * 13.0)), 8.0);
  float field = smoothstep(.18, .92, storm + veins*.18);
  vec3 col = palette(uState, field);

  // Internal lightning responds harder and faster to assistant output.
  float lightning = pow(max(0.0, 1.0 - abs(storm - .52) * 20.0), 2.0);
  lightning *= (.05 + activity * .75);
  col += lightning * mix(vec3(.3,.9,1.0), vec3(1.0,.55,1.0), step(5.5,uState));

  vec3 normal = normalize(vec3(p, sqrt(max(.03, 1.0-dot(p,p)))));
  float light = max(.0, dot(normal, normalize(vec3(-.45,.65,.75))));
  float fresnel = pow(1.0 - max(normal.z, 0.0), 2.4);
  col *= .55 + light * .85;
  col += fresnel * palette(uState, 1.0) * 1.25;
  col += pow(max(0.0, 1.0-distance(p,vec2(-.27,.30))), 10.0) * vec3(.65,.9,1.0);

  float body = 1.0 - smoothstep(.985, 1.015, d);
  float rim = exp(-pow(d - 1.0, 2.0) * 4200.0);
  float aura = exp(-max(0.0, d-1.0) * (7.0-activity*2.0)) * (1.0-body);
  float wave1 = exp(-pow((d - (1.08 + activity*.035)), 2.0) * 850.0);
  float wave2 = exp(-pow((d - (1.18 + activity*.06)), 2.0) * 650.0);
  float listeningWave = (uState > 1.5 && uState < 3.6) ? (.25 + uInput) : 0.0;
  float speakingWave = (uState > 5.5 && uState < 6.5) ? (.2 + uOutput*1.4) : 0.0;
  vec3 glow = palette(uState, 1.0);
  col = col * body + glow * (aura*.28 + rim*.72 + wave1*listeningWave + wave2*speakingWave*.55);

  // Tiny orbiting embers make thinking/delegating feel active.
  float sparks = 0.0;
  for (int i=0; i<7; i++) {
    float fi=float(i), a=t*(.25+fi*.015)+fi*2.39;
    vec2 sp=vec2(cos(a),sin(a))*(1.05+.15*sin(t*.7+fi));
    sparks += exp(-length(uv-sp)*95.0) * step(3.5,uState) * step(uState,5.5);
  }
  col += glow * sparks * 2.2;
  float alpha = clamp(body + rim*.9 + aura*.42 + wave1*(listeningWave+speakingWave) + sparks, 0.0, 1.0);
  fragColor = vec4(col, alpha);
}
`;

const STATES: Record<string, number> = {
  idle: 0,
  connecting: 1,
  listening: 2,
  user_speaking: 3,
  thinking: 4,
  delegating: 5,
  speaking: 6,
  error: 7,
};

/** The uniforms the fragment shader exposes. */
type UniformName = "uResolution" | "uTime" | "uInput" | "uOutput" | "uState" | "uWave[0]";

export class StormOrb {
  readonly element: HTMLElement;
  readonly canvas: HTMLCanvasElement;
  /** null when WebGL2 is unavailable — callers fall back to the CSS orb. */
  gl: WebGL2RenderingContext | null;

  private program: WebGLProgram | null = null;
  private uniforms: Record<UniformName, WebGLUniformLocation | null> | null = null;
  private started = 0;
  private state = 0;
  private input = 0;
  private output = 0;
  private inputTarget = 0;
  private outputTarget = 0;
  private readonly wave = new Float32Array(128);
  private readonly inputWave = new Float32Array(128);
  private readonly outputWave = new Float32Array(128);

  constructor(element: HTMLElement) {
    this.element = element;
    this.canvas = document.createElement("canvas");
    this.canvas.className = "storm-canvas";
    this.canvas.setAttribute("aria-hidden", "true");
    element.prepend(this.canvas);
    this.gl = this.canvas.getContext("webgl2", { alpha: true, antialias: true, premultipliedAlpha: true });
    if (!this.gl) { this._fallback(); return; }
    const gl = this.gl;
    try {
      this.program = this._program(gl);
      this.uniforms = Object.fromEntries(
        (["uResolution", "uTime", "uInput", "uOutput", "uState", "uWave[0]"] as UniformName[])
          .map((n) => [n, gl.getUniformLocation(this.program!, n)]),
      ) as Record<UniformName, WebGLUniformLocation | null>;
      const vertices = gl.createBuffer();
      gl.bindBuffer(gl.ARRAY_BUFFER, vertices);
      gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1, -1, 3, -1, -1, 3]), gl.STATIC_DRAW);
      const position = gl.getAttribLocation(this.program, "position");
      gl.enableVertexAttribArray(position);
      gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0);
    } catch (error) {
      console.warn("Storm orb unavailable; using CSS fallback", error);
      this._fallback();
      return;
    }
    this.started = performance.now();
    new ResizeObserver(() => this._resize()).observe(element);
    this._resize();
    this._frame = this._frame.bind(this);
    requestAnimationFrame(this._frame);
  }

  private _fallback(): void {
    this.gl = null;
    this.element.classList.add("orb-webgl-fallback");
    this.canvas.remove();
  }

  private _shader(gl: WebGL2RenderingContext, type: GLenum, source: string): WebGLShader {
    const shader = gl.createShader(type);
    if (!shader) throw new Error("could not create shader");
    gl.shaderSource(shader, source); gl.compileShader(shader);
    if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw new Error(gl.getShaderInfoLog(shader) ?? "shader compile failed");
    return shader;
  }
  private _program(gl: WebGL2RenderingContext): WebGLProgram {
    const p = gl.createProgram();
    if (!p) throw new Error("could not create program");
    gl.attachShader(p, this._shader(gl, gl.VERTEX_SHADER, VERTEX));
    gl.attachShader(p, this._shader(gl, gl.FRAGMENT_SHADER, FRAGMENT));
    gl.linkProgram(p);
    if (!gl.getProgramParameter(p, gl.LINK_STATUS)) throw new Error(gl.getProgramInfoLog(p) ?? "program link failed");
    gl.useProgram(p); return p;
  }
  private _resize(): void {
    if (!this.gl) return;
    const size = this.canvas.getBoundingClientRect();
    const dpr = Math.min(devicePixelRatio || 1, 2);
    this.canvas.width = Math.round(size.width * dpr); this.canvas.height = Math.round(size.height * dpr);
    this.gl.viewport(0, 0, this.canvas.width, this.canvas.height);
  }
  setState(state: string): void { this.state = STATES[state] ?? 0; }
  setInput(level: number, waveform?: ArrayLike<number>): void { this.inputTarget = Math.min(1, level * 7); this._setWave(this.inputWave, waveform); }
  setOutput(level: number, waveform?: ArrayLike<number>): void { this.outputTarget = Math.min(1, level * 5); this._setWave(this.outputWave, waveform); }
  private _setWave(target: Float32Array, samples?: ArrayLike<number>): void {
    if (!samples?.length) return;
    for (let i = 0; i < 128; i++) target[i] = samples[Math.min(samples.length - 1, Math.floor(i * samples.length / 128))]!;
  }
  private _frame(now: number): void {
    const gl = this.gl;
    if (!gl || !this.program || !this.uniforms) return;
    this.input += (this.inputTarget - this.input) * .24;
    this.output += (this.outputTarget - this.output) * .34;
    this.inputTarget *= .91; this.outputTarget *= .88;
    for (let i = 0; i < 128; i++) {
      const target = this.state === 6 ? this.outputWave[i]! : this.inputWave[i]!;
      this.wave[i]! += (target - this.wave[i]!) * .42;
      this.inputWave[i]! *= .86; this.outputWave[i]! *= .86;
    }
    gl.useProgram(this.program);
    gl.uniform2f(this.uniforms.uResolution, this.canvas.width, this.canvas.height);
    gl.uniform1f(this.uniforms.uTime, (now - this.started) / 1000);
    gl.uniform1f(this.uniforms.uInput, this.input); gl.uniform1f(this.uniforms.uOutput, this.output);
    gl.uniform1f(this.uniforms.uState, this.state); gl.uniform1fv(this.uniforms["uWave[0]"], this.wave);
    gl.clearColor(0, 0, 0, 0); gl.clear(gl.COLOR_BUFFER_BIT); gl.drawArrays(gl.TRIANGLES, 0, 3);
    requestAnimationFrame(this._frame);
  }
}
