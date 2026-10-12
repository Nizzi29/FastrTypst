//! Procedural generator for electrical-engineering formulas in Typst math
//! syntax, following German (DIN 1302/1304/5483) conventions: upright `d`
//! (`dif`), upright `j` and `e`, underlined complex quantities.

use std::collections::{BTreeSet, HashMap};

use rand::rngs::StdRng;
use rand::seq::IndexedRandom;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};

/// A typesetting construct a formula can exercise. Used to find and drill
/// the constructs a player is slow at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Tag {
    Fraction,
    Subscript,
    TextSubscript,
    Superscript,
    Root,
    Greek,
    Underline,
    ImagUnit,
    Exponential,
    Function,
    Integral,
    ClosedIntegral,
    Sum,
    Limit,
    Differential,
    Partial,
    Vector,
    Matrix,
    Abs,
    Accent,
    VectorOperator,
    Transform,
    Correspondence,
    Convolution,
    Cases,
    Infinity,
    Relation,
    Text,
}

impl Tag {
    pub fn label(self) -> &'static str {
        match self {
            Tag::Fraction => "fractions",
            Tag::Subscript => "subscripts",
            Tag::TextSubscript => "text subscripts \"eff\"",
            Tag::Superscript => "superscripts",
            Tag::Root => "roots",
            Tag::Greek => "Greek letters",
            Tag::Underline => "underline (complex)",
            Tag::ImagUnit => "upright j",
            Tag::Exponential => "upright e^",
            Tag::Function => "sin/cos/lg/Re/…",
            Tag::Integral => "integrals",
            Tag::ClosedIntegral => "closed integrals",
            Tag::Sum => "sums/products",
            Tag::Limit => "limits",
            Tag::Differential => "dif",
            Tag::Partial => "partial",
            Tag::Vector => "vector arrows",
            Tag::Matrix => "mat/vec",
            Tag::Abs => "|abs|",
            Tag::Accent => "hat/dot accents",
            Tag::VectorOperator => "nabla/rot/div",
            Tag::Transform => "cal(L)/cal(F)",
            Tag::Correspondence => "correspondence ⊶",
            Tag::Convolution => "convolution ast",
            Tag::Cases => "cases",
            Tag::Infinity => "infinity",
            Tag::Relation => "relations/arrows",
            Tag::Text => "quoted text/units",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Topic {
    Circuits,
    Signals,
    Math,
    Electromagnetics,
}

impl Topic {
    pub fn label(self) -> &'static str {
        match self {
            Topic::Circuits => "Circuits & phasors",
            Topic::Signals => "Signals & systems",
            Topic::Math => "Calculus & linear algebra",
            Topic::Electromagnetics => "Electromagnetics",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Formula {
    /// Canonical Typst math source (without the surrounding `$`).
    pub source: String,
    pub tags: BTreeSet<Tag>,
    pub topic: Topic,
    pub complexity: f32,
}

impl Formula {
    fn new(source: String, topic: Topic) -> Self {
        let tags = detect_tags(&source);
        let complexity = source.chars().count() as f32 / 12.0 + tags.len() as f32 * 0.4;
        Self { source, tags, topic, complexity }
    }
}

/// Complexity budget for a difficulty level in `1.0..=10.0`.
pub fn complexity_budget(level: f32) -> f32 {
    2.0 + level * 1.5
}

pub struct Generator {
    rng: StdRng,
}

impl Generator {
    pub fn new() -> Self {
        Self { rng: StdRng::from_os_rng() }
    }

    #[cfg(test)]
    pub fn seeded(seed: u64) -> Self {
        Self { rng: StdRng::seed_from_u64(seed) }
    }

    /// Generates one random formula from any template.
    pub fn any(&mut self, level: f32) -> Formula {
        let (topic, template) = *TEMPLATES.choose(&mut self.rng).unwrap();
        let mut g = Gen { rng: &mut self.rng, level };
        Formula::new(template(&mut g), topic)
    }

    /// Picks a formula suited to `level`, preferring formulas that exercise
    /// constructs with a high `weakness` score.
    pub fn next(&mut self, level: f32, weakness: &HashMap<Tag, f32>, avoid: &str) -> Formula {
        let budget = complexity_budget(level);
        let mut candidates = Vec::new();
        for _ in 0..200 {
            let f = self.any(level);
            if f.complexity <= budget && f.source != avoid {
                candidates.push(f);
                if candidates.len() == 8 {
                    break;
                }
            }
        }
        if candidates.is_empty() {
            return self.any(level);
        }
        let weights: Vec<f32> = candidates
            .iter()
            .map(|f| {
                let weak: f32 = f.tags.iter().map(|t| weakness.get(t).copied().unwrap_or(crate::stats::EXPLORE_WEAKNESS)).sum();
                let weak = weak / (f.tags.len().max(1) as f32).sqrt();
                // Prefer formulas that use most of the budget.
                let fill = (f.complexity / budget).min(1.0);
                (1.5 * weak + 1.5 * fill).exp()
            })
            .collect();
        let total: f32 = weights.iter().sum();
        let mut x = self.rng.random_range(0.0..total);
        for (f, w) in candidates.iter().zip(&weights) {
            if x < *w {
                return f.clone();
            }
            x -= w;
        }
        candidates.pop().unwrap()
    }
}

// ---------------------------------------------------------------------------
// Tag detection

const GREEK: &[&str] = &[
    "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "kappa", "lambda", "mu",
    "nu", "xi", "pi", "rho", "sigma", "tau", "phi", "chi", "psi", "omega", "Gamma", "Delta",
    "Theta", "Lambda", "Phi", "Psi", "Omega",
];

const FUNCTIONS: &[&str] = &[
    "sin", "cos", "tan", "arctan", "sinh", "cosh", "ln", "lg", "log", "exp", "Re", "Im", "det",
];

/// Derives the constructs used in a Typst math source.
pub fn detect_tags(src: &str) -> BTreeSet<Tag> {
    let mut tags = BTreeSet::new();
    let mut in_str = false;
    let mut prev = ' ';
    let mut word = String::new();
    let mut words = Vec::new();
    for c in src.chars() {
        if c == '"' {
            if !in_str {
                match prev {
                    '_' => drop(tags.insert(Tag::TextSubscript)),
                    '(' => {} // op("rot") etc.
                    _ => drop(tags.insert(Tag::Text)),
                }
            }
            in_str = !in_str;
        } else if !in_str {
            match c {
                '/' => drop(tags.insert(Tag::Fraction)),
                '^' => drop(tags.insert(Tag::Superscript)),
                '_' => drop(tags.insert(Tag::Subscript)),
                '|' => drop(tags.insert(Tag::Abs)),
                _ => {}
            }
            if c.is_ascii_alphabetic() || (c == '.' && !word.is_empty()) {
                word.push(c);
            } else if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        }
        prev = c;
    }
    if !word.is_empty() {
        words.push(word);
    }
    if src.contains("<=") || src.contains(">=") || src.contains("->") || src.contains("=>") {
        tags.insert(Tag::Relation);
    }
    if src.contains("upright(j)") {
        tags.insert(Tag::ImagUnit);
    }
    if src.contains("upright(e)^") {
        tags.insert(Tag::Exponential);
    }
    if src.contains("op(\"rot\")") || src.contains("op(\"div\")") || src.contains("op(\"grad\")") {
        tags.insert(Tag::VectorOperator);
    }
    for w in &words {
        let tag = match w.as_str() {
            "frac" => Tag::Fraction,
            "sqrt" | "root" => Tag::Root,
            "underline" => Tag::Underline,
            "integral" | "integral.double" | "integral.triple" => Tag::Integral,
            "integral.cont" | "integral.surf" => Tag::ClosedIntegral,
            "sum" | "product" => Tag::Sum,
            "lim" => Tag::Limit,
            "dif" | "Dif" => Tag::Differential,
            "partial" => Tag::Partial,
            "arrow" => Tag::Vector,
            "mat" | "vec" => Tag::Matrix,
            "abs" | "norm" => Tag::Abs,
            "hat" | "dot" | "tilde" | "overline" => Tag::Accent,
            "nabla" | "laplace" => Tag::VectorOperator,
            "cal" => Tag::Transform,
            "original" | "image" => Tag::Correspondence,
            "ast" => Tag::Convolution,
            "cases" => Tag::Cases,
            "infinity" => Tag::Infinity,
            "approx" | "parallel" | "prop" | "equiv" | "times" => Tag::Relation,
            w if GREEK.contains(&w) => Tag::Greek,
            w if FUNCTIONS.contains(&w) => Tag::Function,
            _ => continue,
        };
        tags.insert(tag);
    }
    // `dot` is both an accent `dot(x)` and the operator `arrow(a) dot arrow(b)`.
    if words.iter().any(|w| w == "dot") && !src.contains("dot(") {
        tags.remove(&Tag::Accent);
        tags.insert(Tag::Relation);
    }
    tags
}

// ---------------------------------------------------------------------------
// Building blocks

/// Whether `s` can be used as fraction numerator/denominator or attachment
/// without being wrapped in parentheses.
fn is_atomic(s: &str) -> bool {
    if s.starts_with('-') {
        return false;
    }
    let mut depth = 0;
    let mut in_str = false;
    for c in s.chars() {
        match c {
            '"' => in_str = !in_str,
            _ if in_str => {}
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ' ' | '+' | '-' | '/' | ',' | '=' | '&' | '!' | '|' if depth == 0 => return false,
            _ => {}
        }
    }
    true
}

/// Whether `s` has a top-level `+` or `-` (and needs visible parentheses
/// when negated or multiplied).
fn is_sum(s: &str) -> bool {
    let mut depth = 0;
    for c in s.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            '+' | '-' if depth == 0 => return true,
            _ => {}
        }
    }
    false
}

/// Wraps `s` in invisible grouping parentheses if needed.
fn grp(s: &str) -> String {
    if is_atomic(s) { s.to_string() } else { format!("({s})") }
}

fn frac(n: &str, d: &str) -> String {
    format!("{}/{}", grp(n), grp(d))
}

fn sup(base: &str, e: &str) -> String {
    format!("{base}^{}", grp(e))
}

fn sub(base: &str, i: &str) -> String {
    format!("{base}_{}", grp(i))
}

const J: &str = "upright(j)";

fn ex(arg: &str) -> String {
    sup("upright(e)", arg)
}

fn ul(x: &str) -> String {
    format!("underline({x})")
}

fn ar(x: &str) -> String {
    format!("arrow({x})")
}

/// `(dif^n f)/(dif x^n)` in DIN notation.
fn deriv(f: &str, x: &str, order: u32) -> String {
    if order == 1 {
        format!("(dif {f})/(dif {x})")
    } else {
        format!("(dif^{order} {f})/(dif {x}^{order})")
    }
}

fn pderiv(f: &str, x: &str, order: u32) -> String {
    if order == 1 {
        format!("(partial {f})/(partial {x})")
    } else {
        format!("(partial^{order} {f})/(partial {x}^{order})")
    }
}

/// Sum with optional limits; `lo`/`hi` may be empty.
fn bigop(op: &str, lo: &str, hi: &str) -> String {
    let mut s = op.to_string();
    if !lo.is_empty() {
        s = sub(&s, lo);
    }
    if !hi.is_empty() {
        s = sup(&s, hi);
    }
    s
}

/// Per-formula random helper.
struct Gen<'r> {
    rng: &'r mut StdRng,
    level: f32,
}

impl Gen<'_> {
    fn pick<'a>(&mut self, xs: &[&'a str]) -> &'a str {
        xs.choose(self.rng).unwrap()
    }

    fn chance(&mut self, p: f64) -> bool {
        self.rng.random_bool(p.clamp(0.0, 1.0))
    }

    fn int(&mut self, lo: i32, hi: i32) -> i32 {
        self.rng.random_range(lo..=hi)
    }

    /// Integer that grows with the level, between `lo` and `hi`.
    fn scaled(&mut self, lo: i32, hi: i32) -> i32 {
        let top = lo + ((hi - lo) as f32 * self.level / 10.0).round() as i32;
        self.int(lo, top.clamp(lo, hi))
    }

    /// Picks one of several alternatives, given as closures.
    fn variant(&mut self, n: usize) -> usize {
        self.rng.random_range(0..n)
    }

    /// Coefficient `c` written in front of `x` (omitting 1).
    fn coef(&mut self, x: &str) -> String {
        match self.int(1, 9) {
            1 => x.to_string(),
            c => format!("{c} {x}"),
        }
    }

    /// Random polynomial in `x` of degree `deg`, highest power first.
    fn poly(&mut self, x: &str, deg: i32) -> String {
        let mut terms = Vec::new();
        for p in (0..=deg).rev() {
            if p < deg && self.chance(0.25) {
                continue;
            }
            let c = self.int(1, 9);
            let pow = match p {
                0 => String::new(),
                1 => x.to_string(),
                _ => format!("{x}^{p}"),
            };
            terms.push(match (c, p) {
                (_, 0) => c.to_string(),
                (1, _) => pow,
                _ => format!("{c} {pow}"),
            });
        }
        terms.join(" + ")
    }

    /// Random expression in the variable `v` with nesting `depth`.
    fn expr(&mut self, v: &str, depth: u32) -> String {
        if depth == 0 {
            return match self.variant(4) {
                0 => v.to_string(),
                1 => self.coef(v),
                2 => format!("{v}^{}", self.int(2, 4)),
                _ => {
                    let p = self.int(2, 3);
                    self.coef(&format!("{v}^{p}"))
                }
            };
        }
        let d = depth - 1;
        match self.variant(7) {
            0 => {
                let n = self.int(2, 3);
                let mut s = self.expr(v, d);
                for _ in 1..n {
                    let op = self.pick(&[" + ", " - "]);
                    s += op;
                    s += &self.expr(v, d);
                }
                s
            }
            1 => {
                // Constant factor times a sub-expression, e.g. `a sin(x)`.
                let c = self.pick(&["a", "b", "2", "3", "k", "A"]);
                let e = self.expr(v, d);
                if is_sum(&e) { format!("{c} ({e})") } else { format!("{c} {e}") }
            }
            2 => frac(&self.expr(v, d), &self.expr(v, d)),
            3 => {
                let f = self.pick(&["sin", "cos", "ln", "sqrt", "sinh"]);
                format!("{f}({})", self.expr(v, d))
            }
            4 => {
                let inner = self.expr(v, d);
                ex(&if is_sum(&inner) { format!("-({inner})") } else { format!("-{inner}") })
            }
            5 => {
                let inner = self.expr(v, d);
                let p = self.int(2, 3);
                if is_atomic(&inner) {
                    format!("{inner}^{p}")
                } else {
                    format!("({inner})^{p}")
                }
            }
            _ => {
                let a = self.pick(&["omega", "omega_0", "2 pi f", "k"]);
                let f = self.pick(&["sin", "cos"]);
                format!("{f}({a} {v})")
            }
        }
    }

    fn depth(&mut self) -> u32 {
        1 + self.scaled(0, 2) as u32
    }

    /// German unit step symbol.
    fn step(&mut self) -> &'static str {
        self.pick(&["sigma", "epsilon"])
    }
}

// ---------------------------------------------------------------------------
// Templates

type Template = fn(&mut Gen) -> String;

const TEMPLATES: &[(Topic, Template)] = &[
    (Topic::Circuits, c_ohm),
    (Topic::Circuits, c_network),
    (Topic::Circuits, c_divider),
    (Topic::Circuits, c_kirchhoff),
    (Topic::Circuits, c_impedance),
    (Topic::Circuits, c_polar),
    (Topic::Circuits, c_power),
    (Topic::Circuits, c_rms),
    (Topic::Circuits, c_time_signal),
    (Topic::Circuits, c_resonance),
    (Topic::Circuits, c_lc),
    (Topic::Circuits, c_transient),
    (Topic::Circuits, c_transfer),
    (Topic::Circuits, c_nodal),
    (Topic::Circuits, c_units),
    (Topic::Signals, s_laplace),
    (Topic::Signals, s_fourier),
    (Topic::Signals, s_correspondence),
    (Topic::Signals, s_correspondence),
    (Topic::Signals, s_convolution),
    (Topic::Signals, s_rational),
    (Topic::Signals, s_ztransform),
    (Topic::Signals, s_fourier_series),
    (Topic::Signals, s_limit_theorems),
    (Topic::Signals, s_energy),
    (Topic::Signals, s_piecewise),
    (Topic::Signals, s_dft),
    (Topic::Signals, s_ode),
    (Topic::Signals, s_delta),
    (Topic::Math, m_function),
    (Topic::Math, m_function),
    (Topic::Math, m_derivative_rules),
    (Topic::Math, m_integral),
    (Topic::Math, m_series),
    (Topic::Math, m_limit),
    (Topic::Math, m_matrix),
    (Topic::Math, m_matrix_ops),
    (Topic::Math, m_vector),
    (Topic::Math, m_complex),
    (Topic::Math, m_partial),
    (Topic::Math, m_ode),
    (Topic::Electromagnetics, e_maxwell_integral),
    (Topic::Electromagnetics, e_maxwell_differential),
    (Topic::Electromagnetics, e_material),
    (Topic::Electromagnetics, e_coulomb),
    (Topic::Electromagnetics, e_force),
    (Topic::Electromagnetics, e_induction),
    (Topic::Electromagnetics, e_potential),
    (Topic::Electromagnetics, e_biot_savart),
    (Topic::Electromagnetics, e_laplace_eq),
    (Topic::Electromagnetics, e_waves),
    (Topic::Electromagnetics, e_flux),
    (Topic::Electromagnetics, e_boundary),
];

// --- Circuits & phasors ----------------------------------------------------

fn c_ohm(g: &mut Gen) -> String {
    let r = g.pick(&["R", "R_1", "R_2", "R_L", "R_i"]);
    match g.variant(5) {
        0 => format!("U = {r} I"),
        1 => format!("I = {}", frac("U", r)),
        2 => format!("P = U I = I^2 {r} = {}", frac("U^2", r)),
        3 => format!("G = {} = {}", frac("1", r), frac("I", "U")),
        _ => format!("{} = {} {}", ul("U"), ul("Z"), ul("I")),
    }
}

fn c_network(g: &mut Gen) -> String {
    let n = g.scaled(2, 5);
    let (sym, inv) = if g.chance(0.5) { ("R", "G") } else { ("L", "") };
    let items: Vec<String> = (1..=n).map(|k| sub(sym, &k.to_string())).collect();
    match g.variant(4) {
        0 => format!("{sym}_\"ges\" = {}", items.join(" + ")),
        1 => {
            let inv_items: Vec<String> = items.iter().map(|x| frac("1", x)).collect();
            format!("{} = {}", frac("1", &format!("{sym}_\"ges\"")), inv_items.join(" + "))
        }
        2 if !inv.is_empty() => {
            format!("{inv}_\"ges\" = {} {}", bigop("sum", "k=1", &n.to_string()), sub(inv, "k"))
        }
        _ => format!("R_1 parallel R_2 = {}", frac("R_1 R_2", "R_1 + R_2")),
    }
}

fn c_divider(g: &mut Gen) -> String {
    match g.variant(4) {
        0 => format!("U_2 = U_1 {}", frac("R_2", "R_1 + R_2")),
        1 => format!("I_1 = I {}", frac("R_2", "R_1 + R_2")),
        2 => format!(
            "{} = {}",
            frac(&sub(&ul("U"), "2"), &sub(&ul("U"), "1")),
            frac(&sub(&ul("Z"), "2"), &format!("{} + {}", sub(&ul("Z"), "1"), sub(&ul("Z"), "2")))
        ),
        _ => format!("U_L = U_0 {}", frac("R_L", "R_i + R_L")),
    }
}

fn c_kirchhoff(g: &mut Gen) -> String {
    let n = g.scaled(3, 6);
    match g.variant(3) {
        0 => {
            let x = g.pick(&["I", "U"]);
            format!("{} {} = 0", bigop("sum", "k=1", "n"), sub(x, "k"))
        }
        1 => {
            let x = g.pick(&["I", "U", "i", "u"]);
            let mut s = sub(x, "1");
            for k in 2..=n {
                let op = g.pick(&[" + ", " - "]);
                s += op;
                s += &sub(x, &k.to_string());
            }
            s + " = 0"
        }
        _ => {
            let mut s = String::from("U_q");
            for k in 1..n {
                s += &format!(" - U_(R {k})");
            }
            s + " = 0"
        }
    }
}

fn c_impedance(g: &mut Gen) -> String {
    let jw = format!("{J} omega");
    match g.variant(7) {
        0 => format!("{}_L = {jw} L", ul("Z")),
        1 => format!("{}_C = {} = -{J} {}", ul("Z"), frac("1", &format!("{jw} C")), frac("1", "omega C")),
        2 => format!("{} = R + {jw} L", ul("Z")),
        3 => format!("{} = R + {J} (omega L - {})", ul("Z"), frac("1", "omega C")),
        4 => format!("{} = {} = G + {J} B", ul("Y"), frac("1", &ul("Z"))),
        5 => format!(
            "{} = {} + {jw} C + {}",
            ul("Y"),
            frac("1", "R"),
            frac("1", &format!("{jw} L"))
        ),
        _ => format!("{} = R + {J} X = |{}| {}", ul("Z"), ul("Z"), ex(&format!("{J} phi_Z"))),
    }
}

fn c_polar(g: &mut Gen) -> String {
    let (q, ph) = *[("U", "phi_u"), ("I", "phi_i"), ("Z", "phi_Z")].choose(g.rng).unwrap();
    match g.variant(6) {
        0 => format!("{} = {q} {}", ul(q), ex(&format!("{J} {ph}"))),
        1 => format!("{} = {q} angle {ph}", ul(q)),
        2 => format!("|{}| = sqrt(R^2 + X^2)", ul("Z")),
        3 => format!("phi = arctan {}", frac("X", "R")),
        4 => format!("{} = cos phi + {J} sin phi", ex(&format!("{J} phi"))),
        _ => format!("hat({}) = hat(U) {}", ul("U"), ex(&format!("{J} phi_u"))),
    }
}

fn c_power(g: &mut Gen) -> String {
    match g.variant(6) {
        0 => format!("{} = {} {}^* = P + {J} Q", ul("S"), ul("U"), ul("I")),
        1 => "S = sqrt(P^2 + Q^2)".to_string(),
        2 => "P = U I cos phi".to_string(),
        3 => "Q = U I sin phi".to_string(),
        4 => format!("cos phi = {}", frac("P", "S")),
        _ => format!("P = {} integral_0^T u(t) i(t) dif t", frac("1", "T")),
    }
}

fn c_rms(g: &mut Gen) -> String {
    let (x, x_) = if g.chance(0.5) { ("U", "u") } else { ("I", "i") };
    match g.variant(3) {
        0 => format!("{x} = sqrt({} integral_0^T {x_}^2(t) dif t)", frac("1", "T")),
        1 => format!("{x}_\"eff\" = {}", frac(&format!("hat({x})"), "sqrt(2)")),
        _ => format!("overline({x_}) = {} integral_0^T {x_}(t) dif t", frac("1", "T")),
    }
}

fn c_time_signal(g: &mut Gen) -> String {
    let (x, x_, ph) = if g.chance(0.5) { ("U", "u", "phi_u") } else { ("I", "i", "phi_i") };
    let f = g.pick(&["cos", "sin"]);
    match g.variant(4) {
        0 => format!("{x_}(t) = hat({x}) {f}(omega t + {ph})"),
        1 => format!("{x_}(t) = sqrt(2) {x} {f}(omega t + {ph})"),
        2 => format!("omega = 2 pi f = {}", frac("2 pi", "T")),
        _ => format!("{x_}(t) = Re{{sqrt(2) {} {}}}", ul(x), ex(&format!("{J} omega t"))),
    }
}

fn c_resonance(g: &mut Gen) -> String {
    match g.variant(5) {
        0 => format!("omega_0 = {}", frac("1", "sqrt(L C)")),
        1 => format!("f_0 = {}", frac("1", "2 pi sqrt(L C)")),
        2 => format!("Q = {} sqrt({})", frac("1", "R"), frac("L", "C")),
        3 => format!("Q = {} = {}", frac("omega_0 L", "R"), frac("1", "omega_0 R C")),
        _ => format!("B = {} = {}", frac("f_0", "Q"), frac("R", "2 pi L")),
    }
}

fn c_lc(g: &mut Gen) -> String {
    match g.variant(5) {
        0 => format!("i_C (t) = C {}", deriv("u_C (t)", "t", 1)),
        1 => format!("u_L (t) = L {}", deriv("i_L (t)", "t", 1)),
        2 => format!(
            "u_C (t) = {} integral_0^t i_C (tau) dif tau + u_C (0)",
            frac("1", "C")
        ),
        3 => format!("W_C = {} C U^2", frac("1", "2")),
        _ => format!("W_L = {} L I^2", frac("1", "2")),
    }
}

fn c_transient(g: &mut Gen) -> String {
    let e = ex("-t/tau");
    match g.variant(4) {
        0 => format!("u_C (t) = U_0 (1 - {e})"),
        1 => format!("i(t) = {} {e}", frac("U_0", "R")),
        2 => "tau = R C".to_string(),
        _ => format!("i_L (t) = {} (1 - {})", frac("U_0", "R"), ex("-(R t)/L")),
    }
}

fn c_transfer(g: &mut Gen) -> String {
    let h = format!("{}({J} omega)", ul("H"));
    let jwrc = format!("{J} omega R C");
    match g.variant(5) {
        0 => format!(
            "{h} = {} = {}",
            frac(&sub(&ul("U"), "2"), &sub(&ul("U"), "1")),
            frac("1", &format!("1 + {jwrc}"))
        ),
        1 => format!("{h} = {}", frac(&jwrc, &format!("1 + {jwrc}"))),
        2 => format!("|{h}| = {}", frac("1", "sqrt(1 + (omega R C)^2)")),
        3 => format!("omega_g = {}", frac("1", "R C")),
        _ => format!("a(omega) = 20 lg |{h}| thin \"dB\""),
    }
}

fn c_nodal(g: &mut Gen) -> String {
    let n = g.scaled(2, 3);
    let rows: Vec<String> = (1..=n)
        .map(|i| {
            (1..=n)
                .map(|k| {
                    let s = format!("G_{i}{k}");
                    if i == k { s } else { format!("-{s}") }
                })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .collect();
    let u: Vec<String> = (1..=n).map(|k| format!("U_{k}")).collect();
    let i: Vec<String> = (1..=n).map(|k| format!("I_(q {k})")).collect();
    format!("mat({}) vec({}) = vec({})", rows.join("; "), u.join(", "), i.join(", "))
}

fn c_units(g: &mut Gen) -> String {
    let u = g.int(2, 24) * 10;
    let r = g.pick(&["100", "220", "470", "1000"]);
    let rv: i32 = r.parse().unwrap();
    match g.variant(3) {
        0 => format!("I = {} = {} approx {} thin \"mA\"", frac("U", "R"), frac(&format!("{u} thin \"V\""), &format!("{r} thin Omega")), (u as f32 * 1000.0 / rv as f32).round()),
        1 => "f = 50 thin \"Hz\", quad omega = 2 pi f approx 314 thin \"s\"^(-1)".to_string(),
        _ => format!("C = {} thin \"nF\", quad L = {} thin \"mH\"", g.int(1, 99) * 10, g.int(1, 50)),
    }
}

// --- Signals & systems -----------------------------------------------------

fn s_laplace(g: &mut Gen) -> String {
    match g.variant(3) {
        0 => format!("X(s) = integral_0^infinity x(t) {} dif t", ex("-s t")),
        1 => "cal(L){x(t)} = X(s)".to_string(),
        _ => format!(
            "x(t) = {} integral_(sigma - {J} infinity)^(sigma + {J} infinity) X(s) {} dif s",
            frac("1", &format!("2 pi {J}")),
            ex("s t")
        ),
    }
}

fn s_fourier(g: &mut Gen) -> String {
    let xjw = format!("X({J} omega)");
    match g.variant(3) {
        0 => format!(
            "{xjw} = integral_(-infinity)^infinity x(t) {} dif t",
            ex(&format!("-{J} omega t"))
        ),
        1 => format!(
            "x(t) = {} integral_(-infinity)^infinity {xjw} {} dif omega",
            frac("1", "2 pi"),
            ex(&format!("{J} omega t"))
        ),
        _ => format!("cal(F){{x(t)}} = {xjw}"),
    }
}

fn s_correspondence(g: &mut Gen) -> String {
    let st = g.step();
    let a = g.pick(&["a", "2", "3", "5", "alpha"]);
    let w = g.pick(&["omega_0", "omega", "2", "3"]);
    let (time, freq) = match g.variant(9) {
        0 => ("delta(t)".to_string(), "1".to_string()),
        1 => (format!("{st}(t)"), frac("1", "s")),
        2 => (format!("t {st}(t)"), frac("1", "s^2")),
        3 => (format!("t^n {st}(t)"), frac("n!", "s^(n+1)")),
        4 => (format!("{} {st}(t)", ex(&format!("-{a} t"))), frac("1", &format!("s + {a}"))),
        5 => (format!("sin({w} t) {st}(t)"), frac(w, &format!("s^2 + {}", sup(w, "2")))),
        6 => (format!("cos({w} t) {st}(t)"), frac("s", &format!("s^2 + {}", sup(w, "2")))),
        7 => (
            format!("{} sin({w} t) {st}(t)", ex(&format!("-{a} t"))),
            frac(w, &format!("(s + {a})^2 + {}", sup(w, "2"))),
        ),
        _ => ("x(t - t_0)".to_string(), format!("X(s) {}", ex("-s t_0"))),
    };
    if g.chance(0.3) {
        format!("cal(L){{{time}}} = {freq}")
    } else {
        format!("{time} original {freq}")
    }
}

fn s_convolution(g: &mut Gen) -> String {
    match g.variant(3) {
        0 => "y(t) = x(t) ast h(t) = integral_(-infinity)^infinity x(tau) h(t - tau) dif tau".into(),
        1 => "y[n] = sum_(k=-infinity)^infinity x[k] h[n - k]".into(),
        _ => "x(t) ast h(t) original X(s) H(s)".to_string(),
    }
}

fn s_rational(g: &mut Gen) -> String {
    let m = g.scaled(0, 3);
    let n = g.scaled(1, 4).max(m + 1);
    let num = g.poly("s", m);
    let den = g.poly("s", n);
    match g.variant(3) {
        0 => format!("H(s) = {} = {}", frac("Y(s)", "X(s)"), frac(&num, &den)),
        1 => format!("G(s) = {}", frac(&num, &den)),
        _ => format!(
            "H(s) = K {}",
            frac(
                &format!("{} (s - s_(0 mu))", bigop("product", "mu=1", "m")),
                &format!("{} (s - s_(infinity nu))", bigop("product", "nu=1", "n"))
            )
        ),
    }
}

fn s_ztransform(g: &mut Gen) -> String {
    let st = g.step();
    match g.variant(3) {
        0 => "X(z) = sum_(n=0)^infinity x[n] z^(-n)".into(),
        1 => format!("a^n {st}[n] original {}", frac("z", "z - a")),
        _ => format!("H(z) = {}", frac("b_0 + b_1 z^(-1)", "1 + a_1 z^(-1)")),
    }
}

fn s_fourier_series(g: &mut Gen) -> String {
    match g.variant(3) {
        0 => format!(
            "x(t) = sum_(k=-infinity)^infinity c_k {}",
            ex(&format!("{J} k omega_0 t"))
        ),
        1 => format!(
            "c_k = {} integral_(-T/2)^(T/2) x(t) {} dif t",
            frac("1", "T"),
            ex(&format!("-{J} k omega_0 t"))
        ),
        _ => format!(
            "x(t) = {} + sum_(k=1)^infinity (a_k cos(k omega_0 t) + b_k sin(k omega_0 t))",
            frac("a_0", "2")
        ),
    }
}

fn s_limit_theorems(g: &mut Gen) -> String {
    match g.variant(2) {
        0 => "lim_(t -> infinity) x(t) = lim_(s -> 0) s X(s)".into(),
        _ => "x(0^+) = lim_(s -> infinity) s X(s)".into(),
    }
}

fn s_energy(g: &mut Gen) -> String {
    match g.variant(2) {
        0 => format!(
            "E = integral_(-infinity)^infinity |x(t)|^2 dif t = {} integral_(-infinity)^infinity |X({J} omega)|^2 dif omega",
            frac("1", "2 pi")
        ),
        _ => format!(
            "P = lim_(T -> infinity) {} integral_(-T)^T |x(t)|^2 dif t",
            frac("1", "2 T")
        ),
    }
}

fn s_piecewise(g: &mut Gen) -> String {
    let a = g.pick(&["A", "1", "U_0", "hat(U)"]);
    match g.variant(3) {
        0 => format!("x(t) = cases({a} & \"für\" |t| <= T/2, 0 & \"sonst\")"),
        1 => "x(t) = cases(1 - abs(t)/T & \"für\" |t| <= T, 0 & \"sonst\")".to_string(),
        _ => format!(
            "u(t) = cases(0 & \"für\" t < 0, {a} (1 - {}) & \"für\" t >= 0)",
            ex("-t/tau")
        ),
    }
}

fn s_dft(g: &mut Gen) -> String {
    match g.variant(2) {
        0 => format!("X[k] = sum_(n=0)^(N-1) x[n] {}", ex(&format!("-{J} {}", frac("2 pi k n", "N")))),
        _ => format!(
            "x[n] = {} sum_(k=0)^(N-1) X[k] {}",
            frac("1", "N"),
            ex(&format!("{J} {}", frac("2 pi k n", "N")))
        ),
    }
}

fn s_ode(g: &mut Gen) -> String {
    let order = g.scaled(1, 3) as u32;
    match g.variant(2) {
        0 => {
            let mut terms = Vec::new();
            for k in (1..=order).rev() {
                terms.push(format!("a_{k} {}", deriv("y(t)", "t", k)));
            }
            terms.push("a_0 y(t)".into());
            format!("{} = b_0 x(t)", terms.join(" + "))
        }
        _ => "y''(t) + 2 D omega_0 y'(t) + omega_0^2 y(t) = omega_0^2 x(t)".into(),
    }
}

fn s_delta(g: &mut Gen) -> String {
    let st = g.step();
    match g.variant(3) {
        0 => "integral_(-infinity)^infinity f(t) delta(t - t_0) dif t = f(t_0)".into(),
        1 => format!("{st}(t) = integral_(-infinity)^t delta(tau) dif tau"),
        _ => format!("delta(t) = {}", deriv(&format!("{st}(t)"), "t", 1)),
    }
}

// --- Calculus & linear algebra ---------------------------------------------

fn m_function(g: &mut Gen) -> String {
    let v = g.pick(&["x", "t"]);
    let d = g.depth();
    let f = g.pick(&["f", "g", "y", "x"]);
    let f = if f == v { "f" } else { f };
    format!("{f}({v}) = {}", g.expr(v, d))
}

fn m_derivative_rules(g: &mut Gen) -> String {
    match g.variant(5) {
        0 => "(u v)' = u' v + u v'".into(),
        1 => format!("({})' = {}", frac("u", "v"), frac("u' v - u v'", "v^2")),
        2 => format!("{} f(g(x)) = f'(g(x)) g'(x)", frac("dif", "dif x")),
        3 => {
            let d = g.depth().min(2);
            let e = g.expr("x", d);
            format!("{} ({e})", frac("dif", "dif x"))
        }
        _ => format!("f'(x_0) = lim_(h -> 0) {}", frac("f(x_0 + h) - f(x_0)", "h")),
    }
}

fn m_integral(g: &mut Gen) -> String {
    match g.variant(4) {
        0 => {
            let d = g.depth().min(2);
            let e = g.expr("x", d);
            let (a, b) = *[("0", "1"), ("a", "b"), ("0", "pi"), ("-1", "1"), ("0", "infinity")]
                .choose(g.rng)
                .unwrap();
            format!("integral_{}^{} {e} dif x", grp(a), grp(b))
        }
        1 => "integral_a^b f(x) dif x = F(b) - F(a)".into(),
        2 => "integral u' v dif x = u v - integral u v' dif x".into(),
        _ => format!("integral {} dif x = ln|x| + C", frac("1", "x")),
    }
}

fn m_series(g: &mut Gen) -> String {
    match g.variant(5) {
        0 => format!(
            "f(x) = sum_(n=0)^infinity {} (x - x_0)^n",
            frac("f^((n))(x_0)", "n!")
        ),
        1 => format!("{} = sum_(n=0)^infinity {}", ex("x"), frac("x^n", "n!")),
        2 => format!(
            "sin x = x - {} + {} - dots",
            frac("x^3", "3!"),
            frac("x^5", "5!")
        ),
        3 => format!("sum_(k=0)^(n-1) q^k = {}", frac("1 - q^n", "1 - q")),
        _ => "(a + b)^n = sum_(k=0)^n binom(n, k) a^(n-k) b^k".into(),
    }
}

fn m_limit(g: &mut Gen) -> String {
    match g.variant(3) {
        0 => format!("lim_(x -> 0) {} = 1", frac("sin x", "x")),
        1 => format!("lim_(n -> infinity) (1 + {})^n = upright(e)", frac("1", "n")),
        _ => {
            let e = g.expr("x", 1);
            format!("lim_(x -> {}) {}", g.pick(&["0", "infinity", "1", "x_0"]), frac(&e, "x"))
        }
    }
}

fn m_matrix(g: &mut Gen) -> String {
    let n = g.scaled(2, 4) as usize;
    let m = if g.chance(0.7) { n } else { g.scaled(2, 4) as usize };
    let numeric = g.chance(0.5);
    let rows: Vec<String> = (1..=n)
        .map(|i| {
            (1..=m)
                .map(|k| if numeric { g.int(-9, 9).to_string() } else { format!("a_{i}{k}") })
                .collect::<Vec<_>>()
                .join(", ")
        })
        .collect();
    let name = g.pick(&["A", "B", "M"]);
    if !numeric && g.chance(0.4) {
        let x: Vec<String> = (1..=m).map(|k| format!("x_{k}")).collect();
        let b: Vec<String> = (1..=n).map(|k| format!("b_{k}")).collect();
        format!("mat({}) vec({}) = vec({})", rows.join("; "), x.join(", "), b.join(", "))
    } else {
        format!("{name} = mat({})", rows.join("; "))
    }
}

fn m_matrix_ops(g: &mut Gen) -> String {
    match g.variant(5) {
        0 => format!("A^(-1) = {} mat(d, -b; -c, a)", frac("1", "a d - b c")),
        1 => "det(A - lambda I) = 0".into(),
        2 => format!("A {} = lambda {}", ar("x"), ar("x")),
        3 => "(A B)^T = B^T A^T".into(),
        _ => "det mat(a, b; c, d) = a d - b c".into(),
    }
}

fn m_vector(g: &mut Gen) -> String {
    let (a, b) = (ar("a"), ar("b"));
    match g.variant(5) {
        0 => format!("{a} dot {b} = |{a}| |{b}| cos alpha"),
        1 => format!("|{a}| = sqrt(a_x^2 + a_y^2 + a_z^2)"),
        2 => format!(
            "{a} times {b} = vec(a_y b_z - a_z b_y, a_z b_x - a_x b_z, a_x b_y - a_y b_x)"
        ),
        3 => {
            let v: Vec<String> = (0..3).map(|_| g.int(-9, 9).to_string()).collect();
            let w: Vec<String> = (0..3).map(|_| g.int(-9, 9).to_string()).collect();
            format!("{a} = vec({}), quad {b} = vec({})", v.join(", "), w.join(", "))
        }
        _ => format!("{} = {}", sub(&ar("e"), "a"), frac(&a, &format!("|{a}|"))),
    }
}

fn m_complex(g: &mut Gen) -> String {
    match g.variant(5) {
        0 => format!("z = x + {J} y = r {}", ex(&format!("{J} phi"))),
        1 => format!("z^* = x - {J} y"),
        2 => "|z| = sqrt(x^2 + y^2)".into(),
        3 => format!("z_1 z_2 = r_1 r_2 {}", ex(&format!("{J} (phi_1 + phi_2)"))),
        _ => format!("Re{{z}} = {}, quad Im{{z}} = {}", frac("z + z^*", "2"), frac("z - z^*", &format!("2 {J}"))),
    }
}

fn m_partial(g: &mut Gen) -> String {
    match g.variant(4) {
        0 => format!("f_(x x) = {}", pderiv("f", "x", 2)),
        1 => format!("{} = {}", frac("partial^2 f", "partial x partial y"), frac("partial^2 f", "partial y partial x")),
        2 => format!(
            "op(\"grad\") f = vec({}, {}, {})",
            pderiv("f", "x", 1),
            pderiv("f", "y", 1),
            pderiv("f", "z", 1)
        ),
        _ => format!(
            "dif f = {} dif x + {} dif y",
            pderiv("f", "x", 1),
            pderiv("f", "y", 1)
        ),
    }
}

fn m_ode(g: &mut Gen) -> String {
    match g.variant(3) {
        0 => format!("y' + a y = 0 => y(x) = C {}", ex("-a x")),
        1 => "lambda^2 + 2 delta lambda + omega_0^2 = 0".to_string(),
        _ => "lambda_(1,2) = -delta plus.minus sqrt(delta^2 - omega_0^2)".to_string(),
    }
}

// --- Electromagnetics ------------------------------------------------------

fn e_maxwell_integral(g: &mut Gen) -> String {
    let (e, h, d, b, j) = (ar("E"), ar("H"), ar("D"), ar("B"), ar("J"));
    let (ds, da) = (format!("dif {}", ar("s")), format!("dif {}", ar("A")));
    match g.variant(4) {
        0 => format!(
            "integral.cont_(partial A) {h} dot {ds} = integral_A ({j} + {}) dot {da}",
            pderiv(&d, "t", 1)
        ),
        1 => format!(
            "integral.cont_(partial A) {e} dot {ds} = -{} integral_A {b} dot {da}",
            frac("dif", "dif t")
        ),
        2 => format!("integral.surf_(partial V) {d} dot {da} = integral_V rho dif V"),
        _ => format!("integral.surf_(partial V) {b} dot {da} = 0"),
    }
}

fn e_maxwell_differential(g: &mut Gen) -> String {
    let (e, h, d, b, j) = (ar("E"), ar("H"), ar("D"), ar("B"), ar("J"));
    let german = g.chance(0.5);
    let (rot, div) = if german {
        ("op(\"rot\")", "op(\"div\")")
    } else {
        ("nabla times", "nabla dot")
    };
    match g.variant(4) {
        0 => format!("{rot} {h} = {j} + {}", pderiv(&d, "t", 1)),
        1 => format!("{rot} {e} = -{}", pderiv(&b, "t", 1)),
        2 => format!("{div} {d} = rho"),
        _ => format!("{div} {b} = 0"),
    }
}

fn e_material(g: &mut Gen) -> String {
    match g.variant(4) {
        0 => format!("{} = epsilon_0 epsilon_r {}", ar("D"), ar("E")),
        1 => format!("{} = mu_0 mu_r {}", ar("B"), ar("H")),
        2 => format!("{} = kappa {}", ar("J"), ar("E")),
        _ => format!("{} = epsilon_0 {} + {}", ar("D"), ar("E"), ar("P")),
    }
}

fn e_coulomb(g: &mut Gen) -> String {
    let er = sub(&ar("e"), "r");
    match g.variant(3) {
        0 => format!("{} = {} {er}", ar("F"), frac("Q_1 Q_2", "4 pi epsilon_0 r^2")),
        1 => format!("{} = {} {er}", ar("E"), frac("Q", "4 pi epsilon_0 r^2")),
        _ => format!("phi(r) = {}", frac("Q", "4 pi epsilon_0 r")),
    }
}

fn e_force(g: &mut Gen) -> String {
    match g.variant(2) {
        0 => format!("{} = q ({} + {} times {})", ar("F"), ar("E"), ar("v"), ar("B")),
        _ => format!("{} = I ({} times {})", ar("F"), ar("l"), ar("B")),
    }
}

fn e_induction(g: &mut Gen) -> String {
    match g.variant(4) {
        0 => format!("u_\"ind\" = -N {}", deriv("Phi", "t", 1)),
        1 => format!("Phi = integral_A {} dot dif {}", ar("B"), ar("A")),
        2 => format!("L = {}", frac("N Phi", "I")),
        _ => format!("L = {}, quad R_m = {}", frac("N^2", "R_m"), frac("l", "mu A")),
    }
}

fn e_potential(g: &mut Gen) -> String {
    let (e, d) = (ar("E"), ar("D"));
    match g.variant(5) {
        0 => format!("{e} = -op(\"grad\") phi = -nabla phi"),
        1 => format!("U_12 = integral_1^2 {e} dot dif {} = phi_1 - phi_2", ar("s")),
        2 => format!("C = {} = {}", frac("Q", "U"), frac("epsilon A", "d")),
        3 => format!("w_e = {} {e} dot {d}", frac("1", "2")),
        _ => format!("W_e = {} integral_V {e} dot {d} dif V", frac("1", "2")),
    }
}

fn e_biot_savart(g: &mut Gen) -> String {
    let r = ar("r");
    match g.variant(3) {
        0 => format!(
            "dif {} = {} {}",
            ar("H"),
            frac("I", "4 pi"),
            frac(&format!("dif {} times {}", ar("s"), sub(&ar("e"), "r")), "r^2")
        ),
        1 => format!("H = {}", frac("I", "2 pi r")),
        _ => format!(
            "{}({r}) = {} integral_V {} dif V'",
            ar("A"),
            frac("mu_0", "4 pi"),
            frac(&format!("{}({r}')", ar("J")), &format!("|{r} - {r}'|"))
        ),
    }
}

fn e_laplace_eq(g: &mut Gen) -> String {
    match g.variant(2) {
        0 => format!("Delta phi = -{}", frac("rho", "epsilon")),
        _ => format!(
            "nabla^2 phi = {} + {} + {}",
            pderiv("phi", "x", 2),
            pderiv("phi", "y", 2),
            pderiv("phi", "z", 2)
        ),
    }
}

fn e_waves(g: &mut Gen) -> String {
    let e = ar("E");
    match g.variant(4) {
        0 => format!("nabla^2 {e} - mu epsilon {} = 0", pderiv(&e, "t", 2)),
        1 => format!("c_0 = {}", frac("1", "sqrt(mu_0 epsilon_0)")),
        2 => format!("Z_0 = sqrt({}) approx 377 thin Omega", frac("mu_0", "epsilon_0")),
        _ => format!("k = omega sqrt(mu epsilon) = {}", frac("2 pi", "lambda")),
    }
}

fn e_flux(g: &mut Gen) -> String {
    match g.variant(3) {
        0 => format!("{} = {} times {}", ar("S"), ar("E"), ar("H")),
        1 => format!("P = integral.surf_A {} dot dif {}", ar("S"), ar("A")),
        _ => format!("nabla dot {} + {} = 0", ar("J"), pderiv("rho", "t", 1)),
    }
}

fn e_boundary(g: &mut Gen) -> String {
    let n = ar("n");
    match g.variant(3) {
        0 => format!("{n} times ({} - {}) = 0", sub(&ar("E"), "2"), sub(&ar("E"), "1")),
        1 => format!("{n} dot ({} - {}) = sigma", sub(&ar("D"), "2"), sub(&ar("D"), "1")),
        _ => "E_(t 1) = E_(t 2), quad D_(n 2) - D_(n 1) = sigma".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine;

    #[test]
    fn atomicity() {
        assert!(is_atomic("x"));
        assert!(is_atomic("sqrt(a + b)"));
        assert!(is_atomic("underline(U)_2"));
        assert!(is_atomic("R_\"ges\""));
        assert!(!is_atomic("a b"));
        assert!(!is_atomic("-x"));
        assert!(!is_atomic("n!"));
    }

    #[test]
    fn tags() {
        let t = detect_tags("underline(U) = R_\"ges\" upright(j) omega integral.cont x dif t");
        for tag in [Tag::Underline, Tag::TextSubscript, Tag::ImagUnit, Tag::Greek, Tag::ClosedIntegral, Tag::Differential] {
            assert!(t.contains(&tag), "{tag:?}");
        }
        assert!(!t.contains(&Tag::Integral));
    }

    /// Every template must produce valid Typst at every level.
    #[test]
    fn all_templates_compile() {
        let mut generator = Generator::seeded(7);
        let mut failures = Vec::new();
        for level in 1..=10 {
            for _ in 0..150 {
                let f = generator.any(level as f32);
                if let Err(e) = engine::render(&f.source, &engine::Style::default()) {
                    failures.push(format!("{}  =>  {e}", f.source));
                }
            }
        }
        failures.sort();
        failures.dedup();
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn every_template_reachable_and_tagged() {
        let mut generator = Generator::seeded(1);
        for (_, t) in TEMPLATES {
            for _ in 0..20 {
                let mut g = Gen { rng: &mut generator.rng, level: 10.0 };
                let f = Formula::new(t(&mut g), Topic::Math);
                assert!(!f.source.contains("^ "), "{}", f.source);
            }
        }
    }

    #[test]
    #[ignore = "prints samples for manual inspection"]
    fn print_samples() {
        let mut generator = Generator::seeded(3);
        for level in [1.0, 4.0, 7.0, 10.0] {
            println!("--- level {level} (budget {})", complexity_budget(level));
            for _ in 0..12 {
                let f = generator.next(level, &HashMap::new(), "");
                println!("{:5.1}  {}", f.complexity, f.source);
            }
        }
    }
}
