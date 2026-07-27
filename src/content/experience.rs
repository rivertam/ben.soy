pub struct Role {
    /// Year span stamped in the margin rail.
    pub span: &'static str,
    pub title: &'static str,
    pub org: &'static str,
    pub place: &'static str,
    /// The full date range as listed.
    pub dates: &'static str,
    /// What the work actually was, one line per point. Empty for internships.
    pub bullets: &'static [&'static str],
    /// The tools of the era, rendered as chips. Built with the semantic
    /// constructors below: [`language`], [`library`], [`discipline`],
    /// [`tool`].
    pub stack: &'static [Tech],
}

/// What kind of thing a chip names. Purely semantic — the résumé page
/// decides what each kind looks like.
#[derive(Clone, Copy)]
pub enum TechKind {
    Language,
    Library,
    Discipline,
    Tool,
}

/// One chip: a name, what kind of thing it is, and an optional project
/// page for names a reader might not know on sight. Build with
/// [`language`], [`library`], [`discipline`], or [`tool`]; chain
/// [`Tech::at`] to link it.
pub struct Tech {
    pub name: &'static str,
    pub kind: TechKind,
    pub href: Option<&'static str>,
}

impl Tech {
    const fn new(kind: TechKind, name: &'static str) -> Self {
        Tech {
            name,
            kind,
            href: None,
        }
    }

    /// Attach the project page: `library("DBOS").at("https://www.dbos.dev")`.
    pub const fn at(self, url: &'static str) -> Self {
        Tech {
            name: self.name,
            kind: self.kind,
            href: Some(url),
        }
    }
}

pub const fn language(name: &'static str) -> Tech {
    Tech::new(TechKind::Language, name)
}

pub const fn library(name: &'static str) -> Tech {
    Tech::new(TechKind::Library, name)
}

pub const fn discipline(name: &'static str) -> Tech {
    Tech::new(TechKind::Discipline, name)
}

pub const fn tool(name: &'static str) -> Tech {
    Tech::new(TechKind::Tool, name)
}

pub static ROLES: [Role; 6] = [
    Role {
        span: "2024–now",
        title: "Co-founder, Executive Software Lead / Board Member",
        org: "DigiChem",
        place: "New York",
        dates: "Aug 2024 – present",
        bullets: &[
            "Co-founded a chemical synthesis startup with two chemists",
            "Raised a seed round with MVP",
            "Lead and mentored a team of three engineers building a custom ERP/MRP for novel \
             chemical synthesis and optimization from zero to one, along with a handful of \
             domain-oriented pivots",
            "Product powered by LLM-powered durable workflows",
        ],
        stack: &[
            language("TypeScript"),
            library("React Router").at("https://reactrouter.com"),
            library("Prisma").at("https://www.prisma.io"),
            library("DBOS").at("https://www.dbos.dev"),
            tool("SQL"),
            tool("Railway").at("https://railway.com"),
            tool("AWS"),
            tool("GitHub Actions"),
            tool("Graphite").at("https://graphite.dev"),
        ],
    },
    Role {
        span: "2017–2023",
        title: "Software Engineer",
        org: "Standard Bots",
        place: "New York / hybrid",
        dates: "Sep 2017 – Mar 2023",
        bullets: &[
            "First dedicated software engineer; left a post-Series A company of ten-plus \
             engineers and twenty employees",
            "Built and maintained the robotics engine — kinematics, dynamics, control systems, \
             motion planning, vision",
            "Built the platform that runs the robots: a React remote control, a real-time server, \
             WebRTC, ThreeJS, WebAssembly",
        ],
        stack: &[
            language("C++"),
            language("TypeScript"),
            language("Rust"),
            library("React"),
            tool("Linux"),
            tool("WebRTC"),
            tool("Firebase"),
            tool("Docker"),
            library("ROS").at("https://www.ros.org"),
        ],
    },
    Role {
        span: "2015–2016",
        title: "Software Engineer / Lead Engineer",
        org: "A Plus",
        place: "New York",
        dates: "Jun 2015 – Dec 2016",
        bullets: &[
            "Joined a team of 6",
            "As the company dwindled, became lead and only engineer, running the entire site's \
             engineering",
        ],
        stack: &[
            language("Ruby"),
            language("JavaScript"),
            library("Ruby on Rails"),
            library("React"),
            tool("AWS"),
        ],
    },
    Role {
        span: "2014",
        title: "Software Engineering Intern",
        org: "Wolverine Trading",
        place: "Chicago",
        dates: "Summer 2014",
        bullets: &[
            "Fun summer during which I learned how to drink",
            "Good environment but low-impact and not intellectually stimulating",
            "While I already knew a good amount about finance, they did have us take an options \
             trading course in which I learned a lot",
        ],
        stack: &[],
    },
    Role {
        span: "2012",
        title: "Software Engineering Intern",
        org: "Royal Caribbean",
        place: "",
        dates: "Summer 2012",
        bullets: &["Kinda regret this one. Cruises are terrible for the environment!"],
        stack: &[],
    },
    Role {
        span: "2009",
        title: "Research Intern",
        org: "Jackson Memorial Hospital",
        place: "",
        dates: "Summer 2009",
        bullets: &[
            "Kinda regret this one too. A lotta mice died, fortunately none directly by my hand.",
        ],
        stack: &[],
    },
];

pub struct School {
    pub span: &'static str,
    pub school: &'static str,
    pub degree: &'static str,
    pub note: &'static str,
}

pub static EDUCATION: School = School {
    span: "2011–2014",
    school: "Washington University in St. Louis",
    degree: "BS, Computer Science",
    note: "Minor in Design.",
};

pub static SKILLS: [Tech; 5] = [
    discipline("Software Design"),
    discipline("Robotics"),
    language("C++"),
    language("TypeScript"),
    language("Rust"),
];
