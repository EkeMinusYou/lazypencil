(async function (request) {
  let gl;
  try {
    const canvas = document.createElement("canvas");
    canvas.width = request.width;
    canvas.height = request.height;
    gl = canvas.getContext("webgl", { alpha: true, premultipliedAlpha: false, preserveDrawingBuffer: true, antialias: false });
    if (!gl) throw Error("Software WebGL 1.0 context is unavailable");
    const extensions = ["OES_standard_derivatives", "EXT_shader_texture_lod", "EXT_frag_depth"].filter(name => gl.getExtension(name));
    const compile = (type, source) => {
      const shader = gl.createShader(type);
      gl.shaderSource(shader, source);
      gl.compileShader(shader);
      if (!gl.getShaderParameter(shader, gl.COMPILE_STATUS)) throw Error(gl.getShaderInfoLog(shader));
      return shader;
    };
    const precision = (request.source.match(/precision\s+(lowp|mediump|highp)\s+float\s*;/) || [null, "highp"])[1];
    const varyings = [...request.source.matchAll(/varying\s+(?:(lowp|mediump|highp)\s+)?vec2\s+(v_uv|v_texCoord|vUv)\s*;/g)];
    const declarations = varyings.map(value => `varying ${value[1] || precision} vec2 ${value[2]};`).join("\n");
    const assignments = varyings.map(value => `${value[2]}=(a_position+1.0)*0.5;`).join("\n");
    const vertex = compile(gl.VERTEX_SHADER, `precision ${precision} float;attribute vec2 a_position;${declarations}\nvoid main(){${assignments}\ngl_Position=vec4(a_position,0.0,1.0);}`);
    const fragment = compile(gl.FRAGMENT_SHADER, request.source);
    const program = gl.createProgram();
    gl.attachShader(program, vertex); gl.attachShader(program, fragment); gl.linkProgram(program);
    if (!gl.getProgramParameter(program, gl.LINK_STATUS)) throw Error(gl.getProgramInfoLog(program));
    gl.useProgram(program);
    const buffer = gl.createBuffer(); gl.bindBuffer(gl.ARRAY_BUFFER, buffer);
    gl.bufferData(gl.ARRAY_BUFFER, new Float32Array([-1,-1,1,-1,-1,1,-1,1,1,-1,1,1]), gl.STATIC_DRAW);
    const position = gl.getAttribLocation(program, "a_position");
    gl.enableVertexAttribArray(position); gl.vertexAttribPointer(position, 2, gl.FLOAT, false, 0, 0);
    let unit = 0, components = 0;
    const own = (object, name) => Object.prototype.hasOwnProperty.call(object, name);
    const arrays = request.uniform_arrays || {};
    const transparent = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGMAAQAABQABDQottAAAAABJRU5ErkJggg==";
    const textures = new Map();
    const textureUnit = async value => {
      if (textures.has(value)) return textures.get(value);
      if (unit >= gl.getParameter(gl.MAX_TEXTURE_IMAGE_UNITS)) throw Error("Too many sampler inputs");
      const image = new Image(); image.src = value; await image.decode();
      const texture = gl.createTexture(); gl.activeTexture(gl.TEXTURE0 + unit); gl.bindTexture(gl.TEXTURE_2D, texture);
      gl.pixelStorei(gl.UNPACK_FLIP_Y_WEBGL, true);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_S, gl.CLAMP_TO_EDGE); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_WRAP_T, gl.CLAMP_TO_EDGE);
      gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MIN_FILTER, gl.LINEAR); gl.texParameteri(gl.TEXTURE_2D, gl.TEXTURE_MAG_FILTER, gl.LINEAR);
      gl.texImage2D(gl.TEXTURE_2D, 0, gl.RGBA, gl.RGBA, gl.UNSIGNED_BYTE, image);
      textures.set(value, unit);
      return unit++;
    };
    const numeric = new Map([
      [gl.FLOAT, [1, "uniform1fv"]], [gl.INT, [1, "uniform1iv"]], [gl.BOOL, [1, "uniform1iv"]],
      [gl.FLOAT_VEC2, [2, "uniform2fv"]], [gl.FLOAT_VEC3, [3, "uniform3fv"]], [gl.FLOAT_VEC4, [4, "uniform4fv"]],
      [gl.INT_VEC2, [2, "uniform2iv"]], [gl.INT_VEC3, [3, "uniform3iv"]], [gl.INT_VEC4, [4, "uniform4iv"]],
      [gl.BOOL_VEC2, [2, "uniform2iv"]], [gl.BOOL_VEC3, [3, "uniform3iv"]], [gl.BOOL_VEC4, [4, "uniform4iv"]],
      [gl.FLOAT_MAT2, [4, "uniformMatrix2fv"]], [gl.FLOAT_MAT3, [9, "uniformMatrix3fv"]], [gl.FLOAT_MAT4, [16, "uniformMatrix4fv"]],
    ]);
    for (let index = 0; index < gl.getProgramParameter(program, gl.ACTIVE_UNIFORMS); index++) {
      const info = gl.getActiveUniform(program, index);
      const location = gl.getUniformLocation(program, info.name);
      const name = info.name.replace(/\[0\]$/, "");
      const array = own(arrays, name);
      if (!own(request.uniforms, name)) throw Error("Missing uniform declaration/value: " + name);
      if (!array && (info.size !== 1 || info.name !== name)) throw Error("Array declaration metadata is missing: " + name);
      if (info.size > 256) throw Error("Uniform array exceeds 256 active elements: " + name);
      let value = request.uniforms[name];
      if (numeric.has(info.type)) {
        const [width, setter] = numeric.get(info.type);
        const count = width * info.size;
        components += count;
        if (components > 4096) throw Error("Shader exceeds 4096 active numeric components");
        if (array) {
          if (value === null) value = Array(count).fill(0);
          if (!Array.isArray(value) || value.length < count || value.length % width !== 0)
            throw Error("Uniform array is shorter than its active linked elements: " + name);
          // The linker can discard an unused tail. Pass only its active prefix.
          value = value.slice(0, count);
        } else if (width === 1) {
          value = [Number(value)];
        }
        if (setter.startsWith("uniformMatrix")) gl[setter](location, false, value);
        else gl[setter](location, value);
      } else if (info.type === gl.SAMPLER_2D) {
        if (array) {
          if (value === null) value = Array(info.size).fill(transparent);
          if (!Array.isArray(value) || value.length < info.size)
            throw Error("Sampler array is shorter than its active linked elements: " + name);
          const units = [];
          for (const image of value.slice(0, info.size)) units.push(await textureUnit(image));
          gl.uniform1iv(location, units);
        } else gl.uniform1i(location, await textureUnit(value));
      } else throw Error("Unsupported uniform type: " + info.name);
    }
    gl.viewport(0, 0, canvas.width, canvas.height); gl.clearColor(0,0,0,0); gl.clear(gl.COLOR_BUFFER_BIT);
    gl.drawArrays(gl.TRIANGLES, 0, 6); gl.finish();
    if (gl.isContextLost() || gl.getError() !== gl.NO_ERROR) throw Error("WebGL draw failed or lost its context");
    const debug = gl.getExtension("WEBGL_debug_renderer_info");
    return { png: canvas.toDataURL("image/png"), extensions, renderer: debug ? gl.getParameter(debug.UNMASKED_RENDERER_WEBGL) : gl.getParameter(gl.RENDERER) };
  } catch (error) {
    return { error: String(error && error.message || error) };
  } finally {
    if (gl) { const lose = gl.getExtension("WEBGL_lose_context"); if (lose) lose.loseContext(); }
  }
})
