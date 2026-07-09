-- Fixture for worrall_SportsTeams.rpt (PUBLIC; worrallbrian/crystal_reports).
-- SYNTHETIC invented teams (nothing real, nothing from samples/). Portable DDL seeds both the SQLite
-- test DB and the postgres DB the Crystal engine re-queries. Table/columns match the report's stored
-- ODBC bindings (view `vw_sports_tabler_output`: id Int32s; team/sport/league/section/division/
-- hometown text). Several sports/leagues so grouping + per-group summaries have real rows to fold.
DROP TABLE IF EXISTS vw_sports_tabler_output;
CREATE TABLE vw_sports_tabler_output (
  id       INTEGER PRIMARY KEY,
  team     TEXT NOT NULL,
  sport    TEXT NOT NULL,
  league   TEXT NOT NULL,
  section  TEXT NOT NULL,
  division TEXT NOT NULL,
  hometown TEXT NOT NULL
);
INSERT INTO vw_sports_tabler_output (id,team,sport,league,section,division,hometown) VALUES
 (1 ,'Arbor Otters'    ,'Basketball','Northern','A','East','Arborville'),
 (2 ,'Azure Falcons'   ,'Basketball','Northern','A','West','Azuria City'),
 (3 ,'Banovia Bears'   ,'Basketball','Northern','B','East','Banovia'),
 (4 ,'Corvana Comets'  ,'Basketball','Southern','A','West','Corvana'),
 (5 ,'Drenn Dragons'   ,'Hockey'    ,'Northern','A','East','Drenland'),
 (6 ,'Eldor Eagles'    ,'Hockey'    ,'Northern','B','West','Eldor'),
 (7 ,'Fenn Foxes'      ,'Hockey'    ,'Southern','A','East','Fennmark'),
 (8 ,'Gravon Griffins' ,'Hockey'    ,'Southern','B','West','Gravonia'),
 (9 ,'Halvir Hawks'    ,'Soccer'    ,'Central' ,'A','East','Halvira'),
 (10,'Iron Ibis'       ,'Soccer'    ,'Central' ,'A','West','Ironvale'),
 (11,'Juniper Jays'    ,'Soccer'    ,'Central' ,'B','East','Juniper'),
 (12,'Kestria Kites'   ,'Soccer'    ,'Central' ,'B','West','Kestria'),
 (13,'Lumar Lynx'      ,'Baseball'  ,'Coastal' ,'A','East','Lumaria'),
 (14,'Marov Marlins'   ,'Baseball'  ,'Coastal' ,'A','West','Marovia'),
 (15,'Norven Nighthawks','Baseball' ,'Coastal' ,'B','East','Norvenia'),
 (16,'Ostmark Ospreys' ,'Baseball'  ,'Coastal' ,'B','West','Ostmark');
