-- Fixture for worrall_AlphaISOsByCountry.rpt (PUBLIC report; worrallbrian/crystal_reports).
--
-- SYNTHETIC data only — invented country-like rows (NOT real ISO 3166, nothing from samples/).
-- Portable DDL: this exact script seeds BOTH the SQLite test DB (cargo test) and the postgres DB
-- that the Crystal engine re-queries via the cross-engine oracle, so both stacks read identical
-- rows. Column names + types MUST match the report's stored bindings (QESession table
-- `countries_all_iso`: id Int32s; name/alpha_2_code/alpha_3_code/numeric_code/internet_cctld text)
-- or the Crystal engine's VerifyDatabase rejects the refresh.
DROP TABLE IF EXISTS countries_all_iso;
CREATE TABLE countries_all_iso (
  id             INTEGER PRIMARY KEY,
  name           TEXT NOT NULL,
  alpha_2_code   TEXT NOT NULL,
  alpha_3_code   TEXT NOT NULL,
  numeric_code   TEXT NOT NULL,
  internet_cctld TEXT
);
INSERT INTO countries_all_iso (id,name,alpha_2_code,alpha_3_code,numeric_code,internet_cctld) VALUES
 (1 ,'Arboria'   ,'AR','ARB','101','.ar'),
 (2 ,'Azuria'    ,'AZ','AZU','102','.az'),
 (3 ,'Banovia'   ,'BA','BAN','111','.ba'),
 (4 ,'Brenland'  ,'BR','BRN','112','.br'),
 (5 ,'Cal300'    ,'CA','CAL','121','.ca'),
 (6 ,'Corvana'   ,'CO','COR','122','.co'),
 (7 ,'Doria'     ,'DO','DOR','131','.do'),
 (8 ,'Drenland'  ,'DR','DRN','132','.dr'),
 (9 ,'Eldoria'   ,'EL','ELD','141','.el'),
 (10,'Estovia'   ,'ES','EST','142','.es'),
 (11,'Fennmark'  ,'FE','FEN','151','.fe'),
 (12,'Florassa'  ,'FL','FLO','152','.fl'),
 (13,'Gravonia'  ,'GR','GRV','161','.gr'),
 (14,'Gorland'   ,'GO','GOR','162','.go'),
 (15,'Halvira'   ,'HA','HAL','171','.ha'),
 (16,'Ironvale'  ,'IR','IRV','181','.ir'),
 (17,'Juniperia' ,'JU','JUN','191','.ju'),
 (18,'Kestria'   ,'KE','KES','201','.ke'),
 (19,'Lumaria'   ,'LU','LUM','211',NULL),
 (20,'Marovia'   ,'MA','MAR','221','.ma'),
 (21,'Norvenia'  ,'NO','NOR','231','.no'),
 (22,'Ostmark'   ,'OS','OST','241','.os'),
 (23,'Pallavia'  ,'PA','PAL','251','.pa'),
 (24,'Quenland'  ,'QU','QUE','261','.qu');
