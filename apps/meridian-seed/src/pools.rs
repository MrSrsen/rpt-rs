//! Fixed, hand-authored data pools: real ISO reference data and synthetic name
//! fragments. Nothing here is PII — company/person names are assembled from
//! neutral fragments. Every list is a `const`/`&[...]`, so iteration order is
//! stable and the corpus is reproducible.

/// A world region: `(code, name)`, top of the geography hierarchy.
pub(crate) const REGIONS: &[(&str, &str)] = &[
    ("AMER", "Americas (North)"),
    ("EMEA", "Europe"),
    ("APAC", "Asia Pacific"),
    ("LATAM", "Latin America"),
    ("MEAF", "Middle East & Africa"),
];

/// A currency: `(code, name, symbol, decimal_places)`.
pub(crate) const CURRENCIES: &[(&str, &str, &str, u8)] = &[
    ("USD", "US Dollar", "$", 2),
    ("EUR", "Euro", "\u{20ac}", 2),
    ("GBP", "Pound Sterling", "\u{a3}", 2),
    ("JPY", "Japanese Yen", "\u{a5}", 0),
    ("CNY", "Chinese Yuan", "\u{a5}", 2),
    ("CHF", "Swiss Franc", "Fr", 2),
    ("CAD", "Canadian Dollar", "$", 2),
    ("AUD", "Australian Dollar", "$", 2),
    ("SGD", "Singapore Dollar", "$", 2),
    ("HKD", "Hong Kong Dollar", "$", 2),
    ("INR", "Indian Rupee", "\u{20b9}", 2),
    ("BRL", "Brazilian Real", "R$", 2),
    ("MXN", "Mexican Peso", "$", 2),
    ("ZAR", "South African Rand", "R", 2),
    ("AED", "UAE Dirham", "\u{62f}.\u{625}", 2),
    ("SAR", "Saudi Riyal", "\u{fdfc}", 2),
    ("SEK", "Swedish Krona", "kr", 2),
    ("NOK", "Norwegian Krone", "kr", 2),
    ("PLN", "Polish Zloty", "z\u{142}", 2),
    ("TRY", "Turkish Lira", "\u{20ba}", 2),
    ("KRW", "South Korean Won", "\u{20a9}", 0),
    ("NZD", "New Zealand Dollar", "$", 2),
    ("DKK", "Danish Krone", "kr", 2),
    ("ILS", "Israeli New Shekel", "\u{20aa}", 2),
];

/// A country: `(iso2, iso3, name, region_code, currency_code)`. ~40 curated,
/// currencies constrained to [`CURRENCIES`]; the Eurozone reuses `EUR`.
pub(crate) const COUNTRIES: &[(&str, &str, &str, &str, &str)] = &[
    ("US", "USA", "United States", "AMER", "USD"),
    ("CA", "CAN", "Canada", "AMER", "CAD"),
    ("DE", "DEU", "Germany", "EMEA", "EUR"),
    ("FR", "FRA", "France", "EMEA", "EUR"),
    ("NL", "NLD", "Netherlands", "EMEA", "EUR"),
    ("IT", "ITA", "Italy", "EMEA", "EUR"),
    ("ES", "ESP", "Spain", "EMEA", "EUR"),
    ("IE", "IRL", "Ireland", "EMEA", "EUR"),
    ("BE", "BEL", "Belgium", "EMEA", "EUR"),
    ("AT", "AUT", "Austria", "EMEA", "EUR"),
    ("PT", "PRT", "Portugal", "EMEA", "EUR"),
    ("FI", "FIN", "Finland", "EMEA", "EUR"),
    ("GR", "GRC", "Greece", "EMEA", "EUR"),
    ("SK", "SVK", "Slovakia", "EMEA", "EUR"),
    ("SI", "SVN", "Slovenia", "EMEA", "EUR"),
    ("HR", "HRV", "Croatia", "EMEA", "EUR"),
    ("GB", "GBR", "United Kingdom", "EMEA", "GBP"),
    ("CH", "CHE", "Switzerland", "EMEA", "CHF"),
    ("SE", "SWE", "Sweden", "EMEA", "SEK"),
    ("NO", "NOR", "Norway", "EMEA", "NOK"),
    ("DK", "DNK", "Denmark", "EMEA", "DKK"),
    ("PL", "POL", "Poland", "EMEA", "PLN"),
    ("TR", "TUR", "Turkey", "EMEA", "TRY"),
    ("MX", "MEX", "Mexico", "LATAM", "MXN"),
    ("BR", "BRA", "Brazil", "LATAM", "BRL"),
    ("CN", "CHN", "China", "APAC", "CNY"),
    ("JP", "JPN", "Japan", "APAC", "JPY"),
    ("SG", "SGP", "Singapore", "APAC", "SGD"),
    ("HK", "HKG", "Hong Kong", "APAC", "HKD"),
    ("IN", "IND", "India", "APAC", "INR"),
    ("AU", "AUS", "Australia", "APAC", "AUD"),
    ("KR", "KOR", "South Korea", "APAC", "KRW"),
    ("NZ", "NZL", "New Zealand", "APAC", "NZD"),
    ("AE", "ARE", "United Arab Emirates", "MEAF", "AED"),
    ("SA", "SAU", "Saudi Arabia", "MEAF", "SAR"),
    ("ZA", "ZAF", "South Africa", "MEAF", "ZAR"),
    ("IL", "ISR", "Israel", "MEAF", "ILS"),
];

/// Customer industry segments.
pub(crate) const INDUSTRIES: &[&str] = &[
    "Automotive",
    "Aerospace & Defense",
    "Consumer Electronics",
    "Apparel & Textiles",
    "Food & Beverage",
    "Pharmaceuticals",
    "Industrial Machinery",
    "Chemicals",
    "Retail & E-commerce",
    "Construction Materials",
    "Energy & Utilities",
    "Agriculture",
    "Medical Devices",
    "Furniture & Home Goods",
    "Telecommunications",
];

/// Units of measure: `(code, name)`.
pub(crate) const UNITS: &[(&str, &str)] = &[
    ("EA", "Each"),
    ("BOX", "Box"),
    ("PLT", "Pallet"),
    ("KG", "Kilogram"),
    ("TON", "Metric Ton"),
    ("CTN", "Carton"),
    ("CS", "Case"),
    ("DRM", "Drum"),
    ("ROL", "Roll"),
    ("SET", "Set"),
    ("M3", "Cubic Metre"),
    ("L", "Litre"),
];

/// Incoterms: `(code, name)`.
pub(crate) const INCOTERMS: &[(&str, &str)] = &[
    ("EXW", "Ex Works"),
    ("FCA", "Free Carrier"),
    ("FAS", "Free Alongside Ship"),
    ("FOB", "Free On Board"),
    ("CFR", "Cost and Freight"),
    ("CIF", "Cost, Insurance and Freight"),
    ("CPT", "Carriage Paid To"),
    ("CIP", "Carriage and Insurance Paid To"),
    ("DAP", "Delivered At Place"),
    ("DPU", "Delivered At Place Unloaded"),
    ("DDP", "Delivered Duty Paid"),
];

/// Shipment modes: `(code, name)`.
pub(crate) const MODES: &[(&str, &str)] = &[
    ("SEA", "Ocean"),
    ("AIR", "Air"),
    ("ROAD", "Road"),
    ("RAIL", "Rail"),
];

/// Service levels: `(code, name, target_transit_days)`.
pub(crate) const SERVICE_LEVELS: &[(&str, &str, i64)] = &[
    ("ECON", "Economy", 21),
    ("STD", "Standard", 10),
    ("EXP", "Express", 4),
    ("PRIO", "Priority", 2),
];

/// Order statuses: `(code, name, sort_order)`.
pub(crate) const ORDER_STATUSES: &[(&str, &str, i64)] = &[
    ("QUOTE", "Quotation", 1),
    ("CONF", "Confirmed", 2),
    ("PICK", "Picking", 3),
    ("SHIP", "Shipped", 4),
    ("CLOSED", "Closed", 5),
    ("CANC", "Cancelled", 6),
];

/// Shipment statuses: `(code, name, sort_order)`.
pub(crate) const SHIPMENT_STATUSES: &[(&str, &str, i64)] = &[
    ("BOOKED", "Booked", 1),
    ("PICKUP", "Picked Up", 2),
    ("TRANSIT", "In Transit", 3),
    ("CUSTOMS", "In Customs", 4),
    ("DELIV", "Delivered", 5),
    ("EXCEPT", "Exception", 6),
];

/// Payment statuses: `(code, name, sort_order)`.
pub(crate) const PAYMENT_STATUSES: &[(&str, &str, i64)] = &[
    ("OPEN", "Open", 1),
    ("PARTIAL", "Partially Paid", 2),
    ("PAID", "Paid", 3),
    ("OVERDUE", "Overdue", 4),
    ("VOID", "Voided", 5),
];

/// Fuel types: `(code, name)`.
pub(crate) const FUEL_TYPES: &[(&str, &str)] = &[
    ("DSL", "Diesel"),
    ("JET", "Jet Fuel"),
    ("BNK", "Bunker Fuel"),
    ("LNG", "Liquefied Natural Gas"),
];

/// Charge types: `(code, name)`.
pub(crate) const CHARGE_TYPES: &[(&str, &str)] = &[
    ("FRT", "Freight"),
    ("FSC", "Fuel Surcharge"),
    ("CUS", "Customs"),
    ("INS", "Insurance"),
    ("HND", "Handling"),
];

/// Facility kinds.
pub(crate) const FACILITY_TYPES: &[&str] = &["Warehouse", "Hub", "Port", "Terminal", "Cross-dock"];

/// Vehicle kinds.
pub(crate) const VEHICLE_TYPES: &[&str] = &[
    "Semi-Trailer",
    "Box Truck",
    "Rail Wagon",
    "Container Ship",
    "Cargo Aircraft",
    "Van",
];

/// Payment methods.
pub(crate) const PAYMENT_METHODS: &[&str] = &[
    "Wire Transfer",
    "Credit Card",
    "ACH",
    "Cheque",
    "Letter of Credit",
];

/// Tracking event kinds.
pub(crate) const EVENT_TYPES: &[&str] = &[
    "Departed Facility",
    "Arrived Facility",
    "Customs Cleared",
    "Out for Delivery",
    "Delivered",
    "Exception",
    "Scanned",
    "Loaded",
];

/// Employee titles.
pub(crate) const TITLES: &[&str] = &[
    "Account Executive",
    "Sales Representative",
    "Operations Manager",
    "Logistics Coordinator",
    "Driver",
    "Warehouse Supervisor",
    "Project Manager",
    "Customer Success Lead",
    "Regional Director",
    "Freight Analyst",
];

/// Customer contact roles.
pub(crate) const CONTACT_ROLES: &[&str] = &[
    "Purchasing",
    "Accounts Payable",
    "Logistics",
    "General Manager",
    "Receiving",
];

/// Given-name fragments for synthetic people.
pub(crate) const FIRST_NAMES: &[&str] = &[
    "Amara", "Liam", "Sofia", "Noah", "Mei", "Arjun", "Elena", "Omar", "Yuki", "Diego", "Ingrid",
    "Kofi", "Hana", "Lucas", "Priya", "Mateo", "Nadia", "Sven", "Aisha", "Tomas", "Lena", "Ravi",
    "Clara", "Hassan", "Farah", "Pieter", "Rosa", "Kenji", "Zara", "Andres", "Maja", "Idris",
    "Bianca", "Viktor", "Leila", "Marco", "Anaya", "Johan", "Sana", "Bruno",
];

/// Family-name fragments for synthetic people.
pub(crate) const LAST_NAMES: &[&str] = &[
    "Okafor",
    "Nakamura",
    "Rossi",
    "Andersen",
    "Kowalski",
    "Silva",
    "Haddad",
    "Kumar",
    "Novak",
    "Bergström",
    "Vermeer",
    "Costa",
    "Fischer",
    "Moreau",
    "Ivanov",
    "Dlamini",
    "Reyes",
    "Larsen",
    "Petrov",
    "Bianchi",
    "Yilmaz",
    "Nguyen",
    "Weber",
    "Santos",
    "Kelly",
    "Brandt",
    "Mensah",
    "Lindqvist",
    "Ferrari",
    "Abadi",
    "Vos",
    "Cruz",
    "Holm",
    "Radic",
    "Osei",
    "Bauer",
    "Marchetti",
    "Sato",
    "Duarte",
    "Keller",
];

/// Company-name head fragments (customers / suppliers).
pub(crate) const COMPANY_HEADS: &[&str] = &[
    "Vanguard",
    "Meridian",
    "Atlas",
    "Northwind",
    "Cascade",
    "Summit",
    "Orion",
    "Pinnacle",
    "Keystone",
    "Beacon",
    "Harbor",
    "Ironwood",
    "Silverline",
    "Evergreen",
    "Cobalt",
    "Crescent",
    "Redwood",
    "Sterling",
    "Trident",
    "Alpine",
    "Quantum",
    "Solstice",
    "Vertex",
    "Zenith",
    "Emerald",
    "Granite",
    "Horizon",
    "Lattice",
    "Nimbus",
    "Onyx",
];

/// Company-name tail fragments.
pub(crate) const COMPANY_TAILS: &[&str] = &[
    "Industries",
    "Trading",
    "Logistics",
    "Supply Co.",
    "Distribution",
    "Group",
    "Manufacturing",
    "Imports",
    "Exports",
    "Enterprises",
    "Partners",
    "Holdings",
    "Systems",
    "Materials",
];

/// Carrier-name head fragments.
pub(crate) const CARRIER_HEADS: &[&str] = &[
    "TransGlobal",
    "BlueWave",
    "Continental",
    "SwiftLine",
    "Pacific",
    "EuroFreight",
    "AeroCargo",
    "IronRail",
    "SeaBridge",
    "RoadRunner",
    "Nordic",
    "Vantage",
    "Zephyr",
    "Cardinal",
];

/// Carrier-name tail fragments.
pub(crate) const CARRIER_TAILS: &[&str] = &[
    "Freight",
    "Lines",
    "Cargo",
    "Logistics",
    "Express",
    "Shipping",
    "Transport",
];

/// Product category division roots (top level of the self-referencing tree).
pub(crate) const CATEGORY_DIVISIONS: &[&str] = &[
    "Industrial Supplies",
    "Consumer Goods",
    "Raw Materials",
    "Electronics",
    "Perishables",
];

/// Product category group fragments (second level).
pub(crate) const CATEGORY_GROUPS: &[&str] = &[
    "Fasteners",
    "Packaging",
    "Tools",
    "Components",
    "Textiles",
    "Beverages",
    "Frozen",
    "Polymers",
    "Metals",
    "Appliances",
];

/// Product noun fragments.
pub(crate) const PRODUCT_NOUNS: &[&str] = &[
    "Bearing",
    "Valve",
    "Cable",
    "Pallet",
    "Container",
    "Pump",
    "Filter",
    "Gasket",
    "Adapter",
    "Bracket",
    "Coupling",
    "Connector",
    "Module",
    "Panel",
    "Sensor",
    "Regulator",
    "Compressor",
    "Actuator",
    "Cartridge",
    "Fitting",
];

/// Product qualifier fragments.
pub(crate) const PRODUCT_QUALIFIERS: &[&str] = &[
    "Heavy-Duty",
    "Compact",
    "Industrial",
    "Precision",
    "Marine-Grade",
    "High-Temp",
    "Stainless",
    "Modular",
    "Reinforced",
    "Standard",
];

/// City-name syllable fragments (assembled into synthetic place names).
pub(crate) const CITY_HEADS: &[&str] = &[
    "Port", "New", "Fort", "Lake", "San", "Mont", "Nord", "West", "East", "Alt", "Grand", "Rio",
    "Val", "Bel", "Sud",
];

/// City-name body fragments.
pub(crate) const CITY_BODIES: &[&str] = &[
    "haven", "field", "burg", "ton", "dale", "port", "mouth", "ford", "bridge", "stad", "grad",
    "ville", "mere", "wick", "holm",
];

/// Province-name adjective fragments.
pub(crate) const PROVINCE_ADJ: &[&str] = &[
    "Northern", "Southern", "Eastern", "Western", "Central", "Upper", "Lower", "Coastal", "Inland",
    "Highland",
];

/// Province-name noun fragments.
pub(crate) const PROVINCE_NOUN: &[&str] = &[
    "Plains",
    "Highlands",
    "Valley",
    "Coast",
    "Basin",
    "Delta",
    "Ridge",
    "Marches",
    "Downs",
    "Reach",
];

/// A small solid-color swatch per entity kind (see [`crate::png`]).
pub(crate) const BLOB_COLORS: &[(&str, [u8; 3])] = &[
    ("product", [0x3a, 0x6e, 0xa5]),
    ("customer", [0x8a, 0x4f, 0x9e]),
    ("carrier", [0xc0, 0x6b, 0x2a]),
    ("employee", [0x2f, 0x8f, 0x6b]),
    ("facility", [0x9e, 0x4a, 0x4a]),
    ("supplier", [0x5a, 0x5f, 0x2f]),
];
