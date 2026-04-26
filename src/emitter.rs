use crate::ast::Program;

#[derive(Debug, Clone)]
pub struct EmittedModule {
    pub text: String,
}

#[derive(Debug, Default)]
pub struct Emitter;

impl Emitter {
    pub fn new() -> Self {
        Self
    }

    pub fn emit(&self, program: &Program) -> EmittedModule {
        let text = program
            .statements
            .iter()
            .map(|statement| statement.raw_text.as_str())
            .collect::<String>();

        EmittedModule { text }
    }
}
