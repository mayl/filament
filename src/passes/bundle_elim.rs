use crate::{
    core::{self, Loc},
    utils::Binding,
};
use itertools::Itertools;
use std::collections::HashMap;

#[derive(Default)]
/// Eliminate the uses of bundles in the component signatures.
/// For each component that mentions bundles in its signature, we generate
/// explicit ports for each index in the bundle.
/// For each invocation of such components, we replace the bundle ports
/// ([core::Port::Bundle] or [core::Port::InvBundle]) with explicit ports.
pub struct BundleElim {
    /// Mapping from name of a monomorphized instance and bundle name to the /// ports generated by the monomorphization of the bundle.
    sig_bundle_map: HashMap<(core::Id, core::Id), Vec<core::Id>>,
    /// Mapping from names of instances to components
    inst_map: HashMap<core::Id, core::Id>,
    /// Mapping from invocations to instances
    inv_map: HashMap<core::Id, core::Id>,
}

impl BundleElim {
    fn sig_from_invoke(&self, invoke: core::Id) -> core::Id {
        let inst = self.inv_map.get(&invoke).unwrap_or_else(|| {
            unreachable!("No instance for invocation `{invoke}'")
        });
        *self.inst_map.get(inst).unwrap_or_else(|| {
            unreachable!("No component for instance `{inst}'")
        })
    }

    /// Compile bundles mentioned in the signature of a component:
    /// - IO bundles are moved into the component body.
    /// - Input bundles generate assignments from the bundle to the port
    /// - Output bundles generate assignments from the port to the bundles
    /// Returns the ports generated from this process and whether the generated ports are inputs.
    fn compile_sig_port(
        p: core::Bundle,
        is_input: bool,
        pre_cmds: &mut Vec<core::Command>,
        post_cmds: &mut Vec<core::Command>,
    ) -> (Vec<Loc<core::PortDef>>, bool) {
        // Add bundle to the top-level commands
        pre_cmds.push(p.clone().into());
        let core::Bundle {
            name: bundle_name,
            typ:
                core::BundleType {
                    idx,
                    len,
                    liveness,
                    bitwidth,
                },
        } = p;
        let len: u64 = len
            .take()
            .try_into()
            .unwrap_or_else(|s| panic!("Failed to concretize `{s}'"));
        // For each index in the bundle, generate a corresponding port
        let ports =
            (0..len)
                .map(|i| {
                    let bind = Binding::new(vec![(
                        *idx.inner(),
                        core::Expr::concrete(i),
                    )]);
                    let liveness =
                        liveness.clone().take().resolve_exprs(&bind).into();
                    // Name of the new port is the bundle name with the index appended
                    let name = Loc::unknown(core::Id::from(format!(
                        "{}_{i}",
                        bundle_name.clone()
                    )));
                    // Generate connection associated with this bundle port's creation.
                    let this_port = core::Port::This(name.clone()).into();
                    let bundle_port = core::Port::bundle(
                        bundle_name.clone(),
                        Loc::unknown(core::Expr::concrete(i).into()),
                    )
                    .into();
                    // Generate assignment for the bundle
                    if is_input {
                        // bundle{i} = this.p
                        pre_cmds.push(core::Command::Connect(
                            core::Connect::new(bundle_port, this_port, None),
                        ))
                    } else {
                        // this.p = bundle{i}
                        post_cmds.push(core::Command::Connect(
                            core::Connect::new(this_port, bundle_port, None),
                        ))
                    };
                    let port = core::PortDef::Port {
                        name,
                        liveness,
                        bitwidth: bitwidth.clone(),
                    };
                    port.into()
                })
                .collect_vec();
        (ports, is_input)
    }

    /// Transform the signature of a monomorphized component and generate any assignments needed to
    /// implement the bundles mentioned in the signature.
    fn sig(
        &mut self,
        sig: core::Signature,
    ) -> (core::Signature, Vec<core::Command>, Vec<core::Command>) {
        // To add before and after the body
        let (mut pre_cmds, mut post_cmds) = (vec![], vec![]);
        let name = *sig.name;
        // Generate ports for each bundle
        let sig = sig.replace_ports(&mut |p, is_input| {
            let pos = p.pos();
            match p.take() {
                p @ core::PortDef::Port { .. } => {
                    (vec![Loc::new(p, pos)], is_input)
                }
                core::PortDef::Bundle(b) => {
                    let b_name = *b.name;
                    let (ports, is_input) = Self::compile_sig_port(
                        b,
                        is_input,
                        &mut pre_cmds,
                        &mut post_cmds,
                    );
                    // Add the transformed signature to the bundle map.
                    self.sig_bundle_map.insert(
                        (name, b_name),
                        ports
                            .iter()
                            .map(|p| *p.inner().name().inner())
                            .collect(),
                    );
                    (ports, is_input)
                }
            }
        });
        (sig, pre_cmds, post_cmds)
    }

    fn bundle_splat_ports(
        &self,
        comp: core::Id,
        bundle: core::Id,
        (start, end): (core::Expr, core::Expr),
    ) -> impl Iterator<Item = Loc<core::Id>> + '_ {
        let renamed =
            &self.sig_bundle_map.get(&(comp, bundle)).unwrap_or_else(|| {
                unreachable!(
                    "Bundle `{}' not found in component `{}'",
                    bundle, comp
                )
            });
        let start: u64 = start.try_into().unwrap();
        let end: u64 = end.try_into().unwrap();
        (start..end).map(|i| Loc::unknown(renamed[i as usize]))
    }

    fn port(
        &self,
        cur_name: core::Id,
        p: Loc<core::Port>,
    ) -> Vec<Loc<core::Port>> {
        let pos = p.pos();
        match p.take() {
            core::Port::Bundle { name, access } => {
                // XXX: Dumb pattern because let-else doesn't drop the borrow
                // from access in the else branch.
                if matches!(access.inner(), core::Access::Index(_)) {
                    return vec![Loc::new(
                        core::Port::bundle(name, access),
                        pos,
                    )];
                }
                let core::Access::Range { start, end } = access.take() else {
                    unreachable!();
                };

                // This is a bundle in the signature
                if self.sig_bundle_map.contains_key(&(cur_name, *name)) {
                    return self
                        .bundle_splat_ports(cur_name, *name, (start, end))
                        .map(|p| Loc::new(core::Port::This(p), pos))
                        .collect_vec();
                }

                // This is a locally bound bundle
                let s: u64 = start.try_into().unwrap();
                let e: u64 = end.try_into().unwrap();
                (s..e)
                    .map(|idx| {
                        core::Port::bundle(
                            name.clone(),
                            Loc::unknown(core::Expr::concrete(idx).into()),
                        )
                        .into()
                    })
                    .collect_vec()
            }
            core::Port::InvBundle {
                invoke,
                port,
                access,
            } => {
                if let core::Access::Index(_) = access.inner() {
                    return vec![Loc::new(
                        core::Port::inv_bundle(invoke, port, access),
                        pos,
                    )];
                }
                let core::Access::Range { start, end } = access.take() else {
                    unreachable!();
                };

                self.bundle_splat_ports(
                    self.sig_from_invoke(*invoke),
                    *port,
                    (start, end),
                )
                .map(|name| {
                    core::Port::InvPort {
                        invoke: invoke.clone(),
                        name,
                    }
                    .into()
                })
                .collect_vec()
            }
            p => vec![Loc::new(p, pos)],
        }
    }

    fn commands(
        &mut self,
        cur_name: core::Id,
        cmds: Vec<core::Command>,
    ) -> Vec<core::Command> {
        cmds.into_iter()
            .map(|cmd| match cmd {
                core::Command::Instance(core::Instance {
                    ref name,
                    ref component,
                    ..
                }) => {
                    // Add instance -> component mapping
                    self.inst_map.insert(**name, **component);
                    cmd
                }
                core::Command::Invoke(mut inv) => {
                    // Add invoke -> instance mapping
                    self.inv_map.insert(*inv.name, *inv.instance);
                    if let Some(ports) = inv.ports {
                        inv.ports = Some(
                            ports
                                .into_iter()
                                .flat_map(|p| self.port(cur_name, p))
                                .collect_vec(),
                        );
                    }
                    inv.into()
                }
                core::Command::Connect(con) => {
                    if matches!(con.dst.inner(), core::Port::InvBundle { .. }) {
                        unimplemented!(
                            "Bundle splatting in connect not yet implemented"
                        )
                    } else {
                        con.into()
                    }
                }
                core::Command::ForLoop(core::ForLoop {
                    idx,
                    start,
                    end,
                    body,
                }) => {
                    let body = self.commands(cur_name, body);
                    core::ForLoop {
                        idx,
                        start,
                        end,
                        body,
                    }
                    .into()
                }
                core::Command::If(core::If { cond, then, alt }) => {
                    let then = self.commands(cur_name, then);
                    let alt = self.commands(cur_name, alt);
                    core::If { cond, then, alt }.into()
                }
                c @ (core::Command::Fsm(_) | core::Command::Bundle(_)) => c,
            })
            .collect_vec()
    }

    /// Tranverse the component and eliminate bundles.
    fn component(&mut self, comp: core::Component) -> core::Component {
        let (sig, mut pre_cmds, post_cmds) = self.sig(comp.sig);
        let body = self.commands(*sig.name, comp.body);
        pre_cmds.extend(body);
        pre_cmds.extend(post_cmds);
        core::Component {
            sig,
            body: pre_cmds,
        }
    }

    /// Monomorphize the program by generate a component for each parameter of each instance.
    pub fn transform(old_ns: core::Namespace) -> core::Namespace {
        let mut pass = Self::default();
        let mut ns = core::Namespace {
            components: Vec::new(),
            ..old_ns
        };

        // For each parameter of each instance, generate a new component
        for comp in old_ns.components {
            ns.components.push(pass.component(comp));
        }
        ns
    }
}
