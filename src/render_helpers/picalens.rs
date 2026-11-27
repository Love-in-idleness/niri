use anyhow::{ensure, Context as _};
use niri_ipc::PicaLensFilter;
use smithay::backend::allocator::Fourcc;
use smithay::backend::renderer::gles::{ffi, link_program, GlesError, GlesRenderer, GlesTexture};
use smithay::backend::renderer::utils::with_renderer_surface_state;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Physical, Scale, Size};

use super::create_texture;

const VERTEX: &str = r#"#version 310 es
void main() {
    vec2 corner = vec2((gl_VertexID << 1) & 2, gl_VertexID & 2);
    gl_Position = vec4(corner * 2.0 - 1.0, 0.0, 1.0);
}"#;
const HEADER: &str = "#version 310 es\nprecision highp float;\nprecision highp int;\n                      uniform sampler2D inputImage;\nuniform vec2 outputSize;\nout vec4 outputColor;\n";

#[derive(Clone, Copy)]
pub struct UpscalePrograms {
    easu: Option<Program>,
    rcas: Option<Program>,
    sgsr: Option<Program>,
}

#[derive(Clone, Copy)]
struct Program {
    id: u32,
    sampler: i32,
    size: i32,
}

impl Program {
    fn compile(gl: &ffi::Gles2, defines: &str, algorithm: &str, pass: &str) -> Option<Self> {
        match unsafe { link_program(gl, VERTEX, &format!("{HEADER}{defines}{algorithm}{pass}")) } {
            Ok(id) => Some(Self {
                id,
                sampler: unsafe { gl.GetUniformLocation(id, c"inputImage".as_ptr()) },
                size: unsafe { gl.GetUniformLocation(id, c"outputSize".as_ptr()) },
            }),
            Err(err) => {
                warn!("PicaLens shader compilation failed: {err}");
                None
            }
        }
    }
}

impl UpscalePrograms {
    pub fn compile(renderer: &mut GlesRenderer) -> Result<Self, GlesError> {
        renderer.with_context(|gl| {
            let fsr = include_str!("picalens-shaders/fsr1.glsl");
            Self {
                easu: Program::compile(
                    gl,
                    "#define FSR_EASU_F\n",
                    fsr,
                    include_str!("picalens-shaders/easu-pass.glsl"),
                ),
                rcas: Program::compile(
                    gl,
                    "#define FSR_RCAS_F\n",
                    fsr,
                    include_str!("picalens-shaders/rcas-pass.glsl"),
                ),
                sgsr: Program::compile(
                    gl,
                    "",
                    include_str!("picalens-shaders/sgsr1.glsl"),
                    include_str!("picalens-shaders/sgsr-pass.glsl"),
                ),
            }
        })
    }

    pub fn render(
        self,
        renderer: &mut GlesRenderer,
        source: &GlesTexture,
        source_size: Size<i32, Physical>,
        destination: Size<i32, Physical>,
        filter: PicaLensFilter,
    ) -> anyhow::Result<GlesTexture> {
        let message = "PicaLens filter is unavailable";
        let (upscale, sharpen) = match filter {
            PicaLensFilter::Fsr => (
                self.easu.context(message)?,
                Some(self.rcas.context(message)?),
            ),
            PicaLensFilter::Sgsr => (self.sgsr.context(message)?, None),
            PicaLensFilter::Linear => {
                anyhow::bail!("PicaLens linear filter does not require a shader")
            }
        };

        let mut limit = 0;
        renderer.with_context(|gl| unsafe { gl.GetIntegerv(ffi::MAX_TEXTURE_SIZE, &mut limit) })?;
        let w = f64::from(destination.w);
        let h = f64::from(destination.h);
        ensure!(
            w > 0. && h > 0. && source_size.w <= limit && source_size.h <= limit,
            "invalid filter geometry"
        );
        let scale = 1_f64
            .min(2. * f64::from(source_size.w) / w)
            .min(2. * f64::from(source_size.h) / h)
            .min((3840. * 2160. / (w * h)).sqrt())
            .min(f64::from(limit) / w.max(h));
        let extent = Size::from(((w * scale).floor() as i32, (h * scale).floor() as i32));
        ensure!(
            extent.w >= source_size.w && extent.h >= source_size.h && extent != source_size,
            "PicaLens GPU filters require an enlarged destination"
        );

        let output = create_texture(renderer, extent, Fourcc::Abgr8888)?;
        let intermediate = if sharpen.is_some() {
            Some(create_texture(renderer, extent, Fourcc::Abgr8888)?)
        } else {
            None
        };

        renderer.with_context(|gl| unsafe {
            let mut fbo = 0;
            let mut draw_fbo = 0;
            let mut viewport = [0; 4];
            let mut program = 0;
            let mut active_texture = 0;
            let mut texture = 0;
            let mut sampler = 0;
            gl.GetIntegerv(ffi::DRAW_FRAMEBUFFER_BINDING, &mut draw_fbo);
            gl.GetIntegerv(ffi::VIEWPORT, viewport.as_mut_ptr());
            gl.GetIntegerv(ffi::CURRENT_PROGRAM, &mut program);
            gl.GetIntegerv(ffi::ACTIVE_TEXTURE, &mut active_texture);
            gl.ActiveTexture(ffi::TEXTURE0);
            gl.GetIntegerv(ffi::TEXTURE_BINDING_2D, &mut texture);
            gl.GetIntegerv(ffi::SAMPLER_BINDING, &mut sampler);
            let blend = gl.IsEnabled(ffi::BLEND) != 0;
            let scissor = gl.IsEnabled(ffi::SCISSOR_TEST) != 0;
            gl.Disable(ffi::BLEND);
            gl.Disable(ffi::SCISSOR_TEST);
            gl.BindSampler(0, 0);
            gl.GenFramebuffers(1, &mut fbo);
            gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, fbo);
            gl.Viewport(0, 0, extent.w, extent.h);

            let pass = |program: Program,
                        input: &GlesTexture,
                        target: &GlesTexture|
             -> anyhow::Result<()> {
                gl.FramebufferTexture2D(
                    ffi::DRAW_FRAMEBUFFER,
                    ffi::COLOR_ATTACHMENT0,
                    ffi::TEXTURE_2D,
                    target.tex_id(),
                    0,
                );
                ensure!(
                    gl.CheckFramebufferStatus(ffi::DRAW_FRAMEBUFFER) == ffi::FRAMEBUFFER_COMPLETE,
                    "incomplete PicaLens shader framebuffer"
                );
                gl.UseProgram(program.id);
                gl.Uniform1i(program.sampler, 0);
                gl.Uniform2f(program.size, extent.w as f32, extent.h as f32);
                gl.BindTexture(ffi::TEXTURE_2D, input.tex_id());
                gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MIN_FILTER, ffi::LINEAR as i32);
                gl.TexParameteri(ffi::TEXTURE_2D, ffi::TEXTURE_MAG_FILTER, ffi::LINEAR as i32);
                gl.DrawArrays(ffi::TRIANGLES, 0, 3);
                ensure!(
                    gl.GetError() == ffi::NO_ERROR,
                    "PicaLens shader draw failed"
                );
                Ok(())
            };
            let result = if let Some((sharpen, intermediate)) = sharpen.zip(intermediate.as_ref()) {
                pass(upscale, source, intermediate)
                    .and_then(|()| pass(sharpen, intermediate, &output))
            } else {
                pass(upscale, source, &output)
            };

            if blend {
                gl.Enable(ffi::BLEND);
            }
            if scissor {
                gl.Enable(ffi::SCISSOR_TEST);
            }
            gl.BindSampler(0, sampler as u32);
            gl.BindTexture(ffi::TEXTURE_2D, texture as u32);
            gl.ActiveTexture(active_texture as u32);
            gl.UseProgram(program as u32);
            gl.Viewport(viewport[0], viewport[1], viewport[2], viewport[3]);
            gl.BindFramebuffer(ffi::DRAW_FRAMEBUFFER, draw_fbo as u32);
            gl.DeleteFramebuffers(1, &fbo);
            result
        })??;
        Ok(output)
    }
}

/// Source texture density after the compositor's viewport crop and scale.
pub fn surface_scale(surface: &WlSurface) -> Option<Scale<f64>> {
    with_renderer_surface_state(surface, |state| {
        let view = state.view()?;
        let scale = f64::from(state.buffer_scale());
        let x = scale * view.src.size.w / f64::from(view.dst.w);
        let y = scale * view.src.size.h / f64::from(view.dst.h);
        let scale = x.min(y);
        (scale.is_finite() && scale > 0.).then(|| Scale::from(scale))
    })?
}
