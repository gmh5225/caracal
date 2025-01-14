use std::collections::HashMap;
use std::io::Write;

use super::cfg::{Cfg, CfgRegular};
use crate::analysis::dataflow::AnalysisState;
use crate::analysis::dataflow::Engine;
use crate::analysis::reentrancy::ReentrancyAnalysis;
use crate::utils::BUILTINS;
use cairo_lang_sierra::extensions::core::{CoreConcreteLibfunc, CoreLibfunc, CoreType};
use cairo_lang_sierra::ids::ConcreteTypeId;
use cairo_lang_sierra::program::{
    Function as SierraFunction, GenStatement, Param, Statement as SierraStatement,
};
use cairo_lang_sierra::program_registry::ProgramRegistry;
use graphviz_rust::dot_generator::*;
use graphviz_rust::dot_structures::*;
use graphviz_rust::printer::{DotPrinter, PrinterContext};

#[derive(Clone, Default)]
pub struct Analyses {
    /// Reentrancy info result
    pub reentrancy: HashMap<usize, AnalysisState<ReentrancyAnalysis>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Type {
    /// External function defined by the user
    External,
    /// View function defined by the user
    View,
    /// Private function defined by the user
    Private,
    /// Constructor function defined by the user
    Constructor,
    /// Event function
    Event,
    /// Function made by the compiler for storage variables
    /// typically address, read, write
    Storage,
    /// Wrapper around an external function made by the compiler
    Wrapper,
    /// Function of the core library
    Core,
    /// Function of a trait with the ABI attribute that does a call contract
    AbiCallContract,
    /// Function of a trait with the ABI attribute that does a library call
    AbiLibraryCall,
    /// L1 handler function
    L1Handler,
}

#[derive(Clone)]
pub struct Function {
    /// Underlying Function data
    data: SierraFunction,
    /// Type of function
    ty: Option<Type>,
    /// The sequence of statements
    statements: Vec<SierraStatement>,
    /// A regular CFG from the statements
    cfg_regular: CfgRegular,
    /// Storage variables read (NOTE it doesn't have vars read using the syscall directly)
    storage_vars_read: Vec<SierraStatement>,
    /// Storage variables written (NOTE it doesn't have vars written using the syscall directly)
    storage_vars_written: Vec<SierraStatement>,
    /// Core functions called
    core_functions_calls: Vec<SierraStatement>,
    /// Private functions called
    private_functions_calls: Vec<SierraStatement>,
    /// Events emitted (NOTE it doesn't have events emitted using the syscall directly)
    events_emitted: Vec<SierraStatement>,
    /// External functions called through an ABI trait (NOTE it doesn't have external functions called using the syscall directly)
    external_functions_calls: Vec<SierraStatement>,
    /// Library functions called through an ABI trait (NOTE it doesn't have library functions called using the syscall directly)
    library_functions_calls: Vec<SierraStatement>,
    /// Analyses results
    analyses: Analyses,
}

impl Function {
    pub fn new(data: SierraFunction, statements: Vec<SierraStatement>) -> Self {
        Function {
            data,
            ty: None,
            statements,
            cfg_regular: CfgRegular::new(),
            storage_vars_read: Vec::new(),
            storage_vars_written: Vec::new(),
            core_functions_calls: Vec::new(),
            private_functions_calls: Vec::new(),
            events_emitted: Vec::new(),
            external_functions_calls: Vec::new(),
            library_functions_calls: Vec::new(),
            analyses: Analyses::default(),
        }
    }

    pub fn name(&self) -> String {
        self.data.id.to_string()
    }

    pub fn ty(&self) -> &Type {
        // At this point is always initialized
        self.ty.as_ref().unwrap()
    }

    pub fn storage_vars_read(&self) -> impl Iterator<Item = &SierraStatement> {
        self.storage_vars_read.iter()
    }

    pub fn storage_vars_written(&self) -> impl Iterator<Item = &SierraStatement> {
        self.storage_vars_written.iter()
    }

    pub fn core_functions_calls(&self) -> impl Iterator<Item = &SierraStatement> {
        self.core_functions_calls.iter()
    }

    pub fn private_functions_calls(&self) -> impl Iterator<Item = &SierraStatement> {
        self.private_functions_calls.iter()
    }

    pub fn events_emitted(&self) -> impl Iterator<Item = &SierraStatement> {
        self.events_emitted.iter()
    }

    pub fn external_functions_calls(&self) -> impl Iterator<Item = &SierraStatement> {
        self.external_functions_calls.iter()
    }

    pub fn library_functions_calls(&self) -> impl Iterator<Item = &SierraStatement> {
        self.library_functions_calls.iter()
    }

    pub fn analyses(&self) -> &Analyses {
        &self.analyses
    }

    /// Function return variables without the builtins
    pub fn returns(&self) -> impl Iterator<Item = &ConcreteTypeId> {
        self.data
            .signature
            .ret_types
            .iter()
            .filter(|r| !BUILTINS.contains(&r.debug_name.clone().unwrap().as_str()))
    }

    /// Function return variables
    pub fn returns_all(&self) -> impl Iterator<Item = &ConcreteTypeId> {
        self.data.signature.ret_types.iter()
    }

    /// Function parameters without the builtins
    pub fn params(&self) -> impl Iterator<Item = &Param> {
        self.data
            .params
            .iter()
            .filter(|p| !BUILTINS.contains(&p.ty.debug_name.clone().unwrap().as_str()))
    }

    /// Function parameters
    pub fn params_all(&self) -> impl Iterator<Item = &Param> {
        self.data.params.iter()
    }

    pub fn get_statements(&self) -> &Vec<SierraStatement> {
        &self.statements
    }

    pub fn get_statements_at(&self, at: usize) -> &[SierraStatement] {
        &self.statements[at..]
    }

    pub fn get_cfg(&self) -> &CfgRegular {
        &self.cfg_regular
    }

    pub fn analyze(
        &mut self,
        functions: &[Function],
        registry: &ProgramRegistry<CoreType, CoreLibfunc>,
    ) {
        self.cfg_regular.analyze(
            &self.statements,
            self.data.entry_point.0,
            functions,
            registry,
            self.name(),
        );
        self.set_meta_informations(functions, registry);
    }

    /// Set the meta informations such as storage variables read, storage variables written, core function called
    /// private function called, events emitted
    fn set_meta_informations(
        &mut self,
        functions: &[Function],
        registry: &ProgramRegistry<CoreType, CoreLibfunc>,
    ) {
        for s in self.statements.iter() {
            if let GenStatement::Invocation(invoc) = s {
                let lib_func = registry
                    .get_libfunc(&invoc.libfunc_id)
                    .expect("Library function not found in the registry");
                if let CoreConcreteLibfunc::FunctionCall(f_called) = lib_func {
                    // We search for the function called in our list of functions to know its type
                    for function in functions {
                        let function_name = function.name();
                        if function_name.as_str()
                            == f_called.function.id.debug_name.as_ref().unwrap()
                        {
                            match function.ty() {
                                Type::Storage => {
                                    if function_name.ends_with("read") {
                                        self.storage_vars_read.push(s.clone());
                                    } else if function_name.ends_with("write") {
                                        self.storage_vars_written.push(s.clone());
                                    }
                                }
                                Type::Event => self.events_emitted.push(s.clone()),
                                Type::Core => self.core_functions_calls.push(s.clone()),
                                Type::Private => self.private_functions_calls.push(s.clone()),
                                Type::AbiCallContract => {
                                    self.external_functions_calls.push(s.clone())
                                }
                                Type::AbiLibraryCall => {
                                    self.library_functions_calls.push(s.clone())
                                }
                                _ => (),
                            }
                            break;
                        }
                    }
                }
            }
        }
    }

    pub fn run_analyses(
        &mut self,
        functions: &[Function],
        registry: &ProgramRegistry<CoreType, CoreLibfunc>,
    ) {
        if self.ty.unwrap() == Type::External {
            let mut reentrancy = Engine::new(&self.cfg_regular, ReentrancyAnalysis);
            reentrancy.run_analysis(functions, registry);
            self.analyses.reentrancy = reentrancy.result().clone();
        }
    }

    pub(super) fn set_ty(&mut self, ty: Type) {
        self.ty = Some(ty);
    }

    /// Write to a file the function's CFG and return the filename
    pub fn cfg_to_dot(&self, cfg: &dyn Cfg) -> String {
        // name for now good enough
        let file_name = format!(
            "{}.dot",
            self.name()
                .split('<')
                .take(1)
                .next()
                .expect("Error when creating the filename")
        )
        .replace("::", "_");
        let mut graph = graph!(di id!(format!("\"{}\"",&file_name)));

        for bb in cfg.get_basic_blocks() {
            let mut ins = String::new();

            bb.get_instructions()
                .iter()
                .for_each(|i| ins.push_str(&format!("{i}\n")));
            let label = format!("\"BB {}\n{}\"", bb.get_id(), ins);
            graph.add_stmt(Stmt::from(node!(bb.get_id();attr!("label",label))));

            for destination in bb.get_outgoing_basic_blocks().iter() {
                graph.add_stmt(Stmt::from(
                    edge!(node_id!(bb.get_id()) => node_id!(destination)),
                ));
            }
        }

        let output = graph.print(&mut PrinterContext::default());
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .open(&file_name)
            .expect("Error when creating file");
        f.write_all(output.as_bytes()).unwrap();

        file_name
    }
}
