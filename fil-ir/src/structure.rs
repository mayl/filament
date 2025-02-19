use super::{
    AddCtx, Bind, Component, Ctx, Expr, ExprIdx, Foldable, Foreign, InfoIdx,
    InstIdx, InvIdx, ParamIdx, PortIdx, Subst, TimeIdx, TimeSub,
};
use fil_ast::Op;
use itertools::Itertools;
use std::fmt;

#[derive(PartialEq, Eq, Hash, Clone)]
/// An interval of time
pub struct Range {
    pub start: TimeIdx,
    pub end: TimeIdx,
}

impl Foldable<ParamIdx, ExprIdx> for Range {
    type Context = Component;

    fn fold_with<F>(&self, ctx: &mut Self::Context, subst_fn: &mut F) -> Self
    where
        F: FnMut(ParamIdx) -> Option<ExprIdx>,
    {
        Range {
            start: self.start.fold_with(ctx, subst_fn),
            end: self.end.fold_with(ctx, subst_fn),
        }
    }
}

#[derive(PartialEq, Eq, Hash, Clone)]
/// The context in which a port was defined.
pub enum PortOwner {
    /// The port is defined in the signature
    Sig { dir: Direction },
    /// The port is defined by an invocation
    Inv {
        inv: InvIdx,
        dir: Direction,
        base: Foreign<Port, Component>,
    },
    /// The port is defined locally.
    /// It does not have a direction because both reading and writing to it is allowed.
    Local,
}

impl PortOwner {
    /// Input on the signature
    pub const fn sig_in() -> Self {
        Self::Sig { dir: Direction::In }
    }

    /// Output on the signature
    pub const fn sig_out() -> Self {
        Self::Sig {
            dir: Direction::Out,
        }
    }

    /// An input port created by an invocation
    pub const fn inv_in(inv: InvIdx, base: Foreign<Port, Component>) -> Self {
        Self::Inv {
            inv,
            dir: Direction::In,
            base,
        }
    }

    /// An output port created by an invocation
    pub const fn inv_out(inv: InvIdx, base: Foreign<Port, Component>) -> Self {
        Self::Inv {
            inv,
            dir: Direction::Out,
            base,
        }
    }
}

impl PortOwner {
    pub fn is_inv_in(&self) -> bool {
        matches!(
            self,
            PortOwner::Inv {
                inv: _,
                dir: Direction::In,
                base: _
            }
        )
    }
}

#[derive(PartialEq, Eq, Hash, Clone, Debug)]
pub enum Direction {
    /// Input port
    In,
    /// Output port
    Out,
}

impl Direction {
    pub fn is_out(&self) -> bool {
        matches!(self, Direction::Out)
    }

    pub fn is_in(&self) -> bool {
        matches!(self, Direction::In)
    }

    pub fn reverse(&self) -> Self {
        match self {
            Direction::In => Direction::Out,
            Direction::Out => Direction::In,
        }
    }
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Direction::In => write!(f, "in"),
            Direction::Out => write!(f, "out"),
        }
    }
}

#[derive(PartialEq, Eq, Hash, Clone)]
/// Duration when the port caries a meaningful value.
/// Equivalent to the bundle type:
/// ```
/// p[N]: for<i> @['G, 'G+i+10]
/// ```
pub struct Liveness {
    pub idxs: Vec<ParamIdx>,
    pub lens: Vec<ExprIdx>,
    pub range: Range,
}

#[derive(PartialEq, Eq, Hash, Clone)]
/// A port tracks its definition and liveness.
/// A port in the IR generalizes both bundles and normal ports.
pub struct Port {
    pub owner: PortOwner,
    pub width: ExprIdx,
    pub live: Liveness,
    pub info: InfoIdx,
}
impl Port {
    /// Check if this is an invoke defined port
    pub fn is_inv(&self) -> bool {
        matches!(self.owner, PortOwner::Inv { .. })
    }

    /// Check if this is an invoke defined port
    pub fn is_inv_in(&self) -> bool {
        matches!(
            self.owner,
            PortOwner::Inv {
                dir: Direction::In,
                ..
            }
        )
    }

    /// Check if this is an invoke defined port
    pub fn is_inv_out(&self) -> bool {
        matches!(
            self.owner,
            PortOwner::Inv {
                dir: Direction::Out,
                ..
            }
        )
    }

    /// Check if this is a signature port
    pub fn is_sig(&self) -> bool {
        matches!(self.owner, PortOwner::Sig { .. })
    }

    /// Check if this is an input port on the signature
    pub fn is_sig_in(&self) -> bool {
        // We check the direction is `out` because the port direction is flipped
        matches!(
            self.owner,
            PortOwner::Sig {
                dir: Direction::Out
            }
        )
    }

    /// Check if this is an output port on the signature
    pub fn is_sig_out(&self) -> bool {
        // We check the direction is `in` because the port direction is flipped
        matches!(self.owner, PortOwner::Sig { dir: Direction::In })
    }

    /// Check if this is a local port
    pub fn is_local(&self) -> bool {
        matches!(self.owner, PortOwner::Local)
    }
}

#[derive(PartialEq, Eq, Hash, Clone)]
/// Represents a port access in bundle syntax since the IR desugars all ports to
/// bundles.
pub struct Access {
    pub port: PortIdx,
    /// Accesses into the bundle (inclusive, exclusive)
    pub ranges: Vec<(ExprIdx, ExprIdx)>,
}
impl Access {
    /// Construct an access on a simple port (i.e. not a bundle)
    /// The access only indexes into the first element of the port.
    pub fn port(port: PortIdx, ctx: &mut impl AddCtx<Expr>) -> Self {
        let zero = ctx.add(Expr::Concrete(0));
        let one = ctx.add(Expr::Concrete(1));
        Self {
            port,
            ranges: vec![(zero, one)],
        }
    }

    fn unit_range(start: ExprIdx, end: ExprIdx, ctx: &Component) -> bool {
        let Some(one) = ctx.exprs().find(&Expr::Concrete(1)) else {
            ctx.internal_error("Constant 1 not found in component")
        };
        match ctx.get(end) {
            Expr::Bin {
                op: Op::Add,
                lhs,
                rhs,
            } => {
                if !(*rhs == one && start == *lhs
                    || *lhs == one && start == *rhs)
                {
                    return false;
                }
            }
            Expr::Concrete(e) => {
                if let Some(s) = start.as_concrete(ctx) {
                    if *e != s + 1 {
                        return false;
                    }
                } else {
                    return false;
                }
            }
            _ => return false,
        }
        true
    }

    /// Check if this is guaranteed a simple port access, i.e., an access that
    /// produces one port.
    /// The check is syntactic and therefore conservative.
    pub fn is_port(&self, ctx: &Component) -> bool {
        self.ranges
            .iter()
            .all(|(start, end)| Self::unit_range(*start, *end, ctx))
    }

    /// Return the bundle type associated with this access
    pub fn bundle_typ(&self, ctx: &mut Component) -> Liveness {
        let live = ctx.get(self.port).live.clone();
        assert!(
            live.idxs.len() == self.ranges.len(),
            "access does not match bundle type dimensions"
        );

        let binding = Bind::new(live.idxs.iter().zip(&self.ranges).map(
            |(idx, (start, end))| {
                if Self::unit_range(*start, *end, ctx) {
                    (*idx, *start)
                } else {
                    (*idx, idx.expr(ctx).add(*start, ctx))
                }
            },
        ));

        let range = Subst::new(live.range, &binding).apply(ctx);
        let lens = self
            .ranges
            .iter()
            .map(|(start, end)| end.sub(*start, ctx))
            .collect_vec();
        // Shrink the bundle type based on the access
        Liveness {
            idxs: live.idxs,
            lens,
            range,
        }
    }
}

#[derive(PartialEq, Eq, Hash, Clone)]
/// Construct that defines the parameter
pub enum ParamOwner {
    /// Defined by the signature (passed in when instantiated)
    Sig,
    /// Owned by an `exists` binding
    Exists {
        /// If this existential parameter should be treated as an
        /// instance-specific parameter
        opaque: bool,
    },
    /// Parameter defined by an instance
    Instance {
        inst: InstIdx,
        base: Foreign<Param, Component>,
    },
    /// Defined by a bundle
    Bundle(PortIdx),
    /// Loop indexing parameter
    Loop,
}

impl fmt::Display for ParamOwner {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ParamOwner::Sig => {
                write!(f, "sig")
            }
            ParamOwner::Exists { opaque: instanced } => {
                write!(f, "{}", if *instanced { "opaque" } else { "some" })
            }
            ParamOwner::Loop => {
                write!(f, "loop")
            }
            ParamOwner::Bundle(idx) => {
                write!(f, "{idx}")
            }
            ParamOwner::Instance { inst: idx, .. } => {
                write!(f, "{idx}")
            }
        }
    }
}

impl ParamOwner {
    pub fn bundle(port: PortIdx) -> Self {
        Self::Bundle(port)
    }
}

#[derive(PartialEq, Eq, Hash, Clone)]
/// Parameters with an optional initial value
pub struct Param {
    pub owner: ParamOwner,
    pub info: InfoIdx,
}

impl Param {
    pub fn new(owner: ParamOwner, info: InfoIdx) -> Self {
        Self { owner, info }
    }

    pub fn is_sig_owned(&self) -> bool {
        matches!(self.owner, ParamOwner::Sig)
    }

    pub fn is_local(&self) -> bool {
        matches!(self.owner, ParamOwner::Loop)
    }
}

impl ParamIdx {
    /// Return an expression that refers to this parameter.
    pub fn expr<C: AddCtx<Expr>>(self, ctx: &mut C) -> ExprIdx {
        ctx.add(Expr::Param(self))
    }
}

#[derive(PartialEq, Eq, Hash, Clone)]
/// Events must have a delay and an optional default value
pub struct Event {
    pub delay: TimeSub,
    pub info: InfoIdx,
    pub has_interface: bool,
}
