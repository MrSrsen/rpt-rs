-- Fixture for worrall_USStatesWithAbbreviations.rpt (PUBLIC; worrallbrian/crystal_reports).
-- SYNTHETIC invented "province" rows (nothing real, nothing from samples/). Portable DDL seeds both
-- the SQLite test DB and the postgres DB the Crystal engine re-queries. Table/columns match the report's
-- stored ODBC bindings (table `provinces_all`: id/country_id Int32s; name_short/name_long text).
-- NOTE: the report's stored RecordSelectionFormula is `{provinces_all.country_id} = 2`, which
-- rpt-render/the Crystal engine push into `WHERE country_id = 2`, so every seeded row uses country_id=2
-- (rows with any other value are filtered out and the report renders empty). Fixtures MUST satisfy
-- the report's record selection.
DROP TABLE IF EXISTS provinces_all;
CREATE TABLE provinces_all (
  id         INTEGER PRIMARY KEY,
  name_short TEXT NOT NULL,
  name_long  TEXT NOT NULL,
  country_id INTEGER NOT NULL
);
INSERT INTO provinces_all (id,name_short,name_long,country_id) VALUES
 (1 ,'AB','Arbor State'    ,2),
 (2 ,'AZ','Azure Province' ,2),
 (3 ,'BN','Banovia'        ,2),
 (4 ,'BR','Brenmark'       ,2),
 (5 ,'CV','Corvana'        ,2),
 (6 ,'DR','Drenland'       ,2),
 (7 ,'EL','Eldor'          ,2),
 (8 ,'FN','Fennmark'       ,2),
 (9 ,'GR','Gravonia'       ,2),
 (10,'HL','Halvira'        ,2),
 (11,'IR','Ironvale'       ,2),
 (12,'JN','Juniper'        ,2),
 (13,'KS','Kestria'        ,2),
 (14,'LM','Lumaria'        ,2),
 (15,'MV','Marovia'        ,2),
 (16,'NV','Norvenia'       ,2),
 (17,'OS','Ostmark'        ,2),
 (18,'PL','Pallavia'       ,2),
 (19,'QN','Quenland'       ,2),
 (20,'RV','Rivenna'        ,2);
