use bevy::render::render_resource::RenderPipelineDescriptor;
use bevy::shader::ShaderDefVal;

fn has(defs: &[ShaderDefVal], name: &str) -> bool {
    defs.iter().any(|d| matches!(d, ShaderDefVal::Bool(n, true) if n == name))
}

pub fn is_normal_prepass(descriptor: &RenderPipelineDescriptor) -> bool {
    has(&descriptor.vertex.shader_defs, "PREPASS_PIPELINE")
        && descriptor.fragment.as_ref().is_some_and(|f| has(&f.shader_defs, "NORMAL_PREPASS"))
}

pub fn specialize_main_pass(descriptor: &mut RenderPipelineDescriptor) {
    if has(&descriptor.vertex.shader_defs, "PREPASS_PIPELINE") {
        return;
    }
    let strip = |defs: &mut Vec<ShaderDefVal>| {
        defs.retain(|d| !matches!(d, ShaderDefVal::Bool(n, _) if n == "LOAD_PREPASS_NORMALS"));
    };
    strip(&mut descriptor.vertex.shader_defs);
    if let Some(f) = descriptor.fragment.as_mut() {
        strip(&mut f.shader_defs);
    }
}
