-- parking.sql — the shared synthetic "airport parking" render-test database.

DROP TABLE IF EXISTS order_service;
DROP TABLE IF EXISTS orders;
DROP TABLE IF EXISTS service;
DROP TABLE IF EXISTS parking_lot;
DROP TABLE IF EXISTS partner;

-- ---- Dimensions ---------------------------------------------------------------------------------

CREATE TABLE partner (
  id          INTEGER      NOT NULL PRIMARY KEY,
  code        VARCHAR(8)   NOT NULL,
  name        VARCHAR(60)  NOT NULL,
  country     VARCHAR(40)  NOT NULL,
  api_enabled BOOLEAN      NOT NULL,
  notes       VARCHAR(400)            -- nullable; long-ish text for wrapping in a partner listing
);

CREATE TABLE parking_lot (
  id       INTEGER      NOT NULL PRIMARY KEY,
  code     VARCHAR(8)   NOT NULL,
  name     VARCHAR(50)  NOT NULL,
  capacity INTEGER      NOT NULL,
  covered  BOOLEAN      NOT NULL,
  address  VARCHAR(120) NOT NULL
);

CREATE TABLE service (
  id         INTEGER     NOT NULL PRIMARY KEY,
  code       VARCHAR(10) NOT NULL,
  name       VARCHAR(50) NOT NULL,
  unit_price DECIMAL(8,2) NOT NULL,
  taxable    BOOLEAN     NOT NULL
);

-- ---- Fact ---------------------------------------------------------------------------------------

CREATE TABLE orders (
  id                INTEGER      NOT NULL PRIMARY KEY,
  order_number      VARCHAR(16)  NOT NULL,
  partner_id        INTEGER,                    -- nullable: direct (non-partner) customers
  parking_lot_id    INTEGER      NOT NULL,
  customer_first    VARCHAR(30)  NOT NULL,
  customer_last     VARCHAR(40)  NOT NULL,
  customer_email    VARCHAR(80)  NOT NULL,
  created_at        TIMESTAMP    NOT NULL,      -- datetime formatting
  arrival_date      DATE         NOT NULL,      -- date formatting + date-group granularity
  departure_date    DATE         NOT NULL,
  arrival_time      TIME,                       -- nullable time formatting
  departure_time    TIME,
  passengers        INTEGER      NOT NULL,
  nights            INTEGER      NOT NULL,
  price_per_night   DECIMAL(8,2) NOT NULL,
  subtotal          DECIMAL(10,2) NOT NULL,
  discount          DECIMAL(10,2) NOT NULL,     -- <= 0; exercises negative-number format
  tax_rate          DECIMAL(5,4) NOT NULL,      -- e.g. 0.2100 — decimal-places format
  tax_amount        DECIMAL(10,2) NOT NULL,
  total             DECIMAL(10,2) NOT NULL,     -- summaries / running totals / charts
  paid              BOOLEAN      NOT NULL,       -- boolean word-pair format
  oversized_vehicle BOOLEAN      NOT NULL,
  state             VARCHAR(12)  NOT NULL,       -- cart | paid | completed | cancelled (grouping)
  source            VARCHAR(8)   NOT NULL,       -- web | partner | phone (cross-tab column)
  note              VARCHAR(500),                -- nullable long text — can-grow / word-wrap
  FOREIGN KEY (partner_id)     REFERENCES partner (id),
  FOREIGN KEY (parking_lot_id) REFERENCES parking_lot (id)
);

-- Order line items (add-on services) — master/detail, subreport, and cross-tab material.
CREATE TABLE order_service (
  id         INTEGER      NOT NULL PRIMARY KEY,
  order_id   INTEGER      NOT NULL,
  service_id INTEGER      NOT NULL,
  quantity   INTEGER      NOT NULL,
  line_total DECIMAL(8,2) NOT NULL,
  FOREIGN KEY (order_id)   REFERENCES orders (id),
  FOREIGN KEY (service_id) REFERENCES service (id)
);

-- ---- Seed ---------------------------------------------------------------------------------------

INSERT INTO partner (id, code, name, country, api_enabled, notes) VALUES
 (1, 'ACME',  'ACME Travel',     'Czechia',  TRUE,  'Corporate account; monthly consolidated invoicing.'),
 (2, 'GLOBE', 'Globe Tours',     'Slovakia', TRUE,  NULL),
 (3, 'CITY',  'City Breaks Ltd', 'Austria',  FALSE, 'Seasonal partner — summer only. This intentionally long note exercises word-wrap and can-grow section height across several lines in the partner listing report.');

INSERT INTO parking_lot (id, code, name, capacity, covered, address) VALUES
 (1, 'P1', 'Terminal North', 250, TRUE,  'Airport Rd 1, North Gate'),
 (2, 'P2', 'Terminal South', 180, FALSE, 'Airport Rd 2, South Gate'),
 (3, 'P3', 'Economy Far',    500, FALSE, 'Ring Rd 15, Sector E');

INSERT INTO service (id, code, name, unit_price, taxable) VALUES
 (1, 'WASH-B', 'Car Wash Basic',   12.00, TRUE),
 (2, 'WASH-P', 'Car Wash Premium', 25.00, TRUE),
 (3, 'LUGG',   'Luggage Packing',   8.50, TRUE),
 (4, 'SEAT-C', 'Child Seat',        5.00, FALSE),
 (5, 'VALET',  'Valet Service',    40.00, TRUE);

INSERT INTO orders
 (id, order_number, partner_id, parking_lot_id, customer_first, customer_last, customer_email,
  created_at, arrival_date, departure_date, arrival_time, departure_time, passengers, nights,
  price_per_night, subtotal, discount, tax_rate, tax_amount, total, paid, oversized_vehicle,
  state, source, note) VALUES
 (1,'ORD-2024-0001',1,   1,'Jan','Novak','jan.novak@example.com','2024-01-03 09:12:00','2024-01-10','2024-01-14','06:30:00','22:15:00',2, 4,15.00, 60.00,  0.00,0.2100,12.60, 72.60,TRUE, FALSE,'completed','partner','Frequent corporate traveler.'),
 (2,'ORD-2024-0002',NULL,2,'Petra','Svobodova','petra.s@example.com','2024-01-05 14:03:00','2024-01-12','2024-01-19','04:45:00','23:50:00',1, 7,12.00, 84.00,  0.00,0.2100,17.64,101.64,TRUE, FALSE,'completed','web',NULL),
 (3,'ORD-2024-0003',2,   3,'Tomas','Dvorak','tomas.dvorak@example.com','2024-01-08 08:20:00','2024-01-15','2024-01-29',NULL,NULL,3,14, 8.00,112.00,-12.00,0.2100,21.00,121.00,TRUE, TRUE, 'completed','partner','Oversized van; assign an end bay.'),
 (4,'ORD-2024-0004',NULL,1,'Eva','Cerna','eva.cerna@example.com','2024-01-20 19:44:00','2024-01-25','2024-01-27','11:00:00','15:30:00',2, 2,15.00, 30.00,  0.00,0.2100, 6.30, 36.30,FALSE,FALSE,'cancelled','phone','Cancelled by customer; refund pending.'),
 (5,'ORD-2024-0005',3,   2,'Marek','Horak','marek.horak@example.com','2024-01-28 12:15:00','2024-02-02','2024-02-09','05:20:00','21:05:00',4, 7,12.00, 84.00,-20.00,0.2100,13.44, 77.44,TRUE, FALSE,'completed','partner',NULL),
 (6,'ORD-2024-0006',1,   1,'Lucie','Mala','lucie.mala@example.com','2024-02-01 07:05:00','2024-02-05','2024-02-15','06:00:00','20:00:00',2,10,15.00,150.00,  0.00,0.2100,31.50,181.50,TRUE, FALSE,'completed','partner','Guest requested an electric charging bay, extra luggage handling, and an early 6am shuttle to the terminal; please confirm one day prior to arrival.'),
 (7,'ORD-2024-0007',NULL,3,'David','Kral','david.kral@example.com','2024-02-06 16:30:00','2024-02-10','2024-02-12',NULL,NULL,1, 2, 8.00, 16.00,  0.00,0.2100, 3.36, 19.36,TRUE, FALSE,'completed','web',NULL),
 (8,'ORD-2024-0008',2,   2,'Hana','Pokorna','hana.pokorna@example.com','2024-02-14 10:10:00','2024-02-20','2024-02-27','08:30:00','09:45:00',2, 7,12.00, 84.00,-12.00,0.2100,15.12, 87.12,TRUE, FALSE,'paid','partner',NULL),
 (9,'ORD-2024-0009',NULL,1,'Ondrej','Benes','ondrej.benes@example.com','2024-02-19 21:22:00','2024-02-24','2024-02-26','13:15:00','17:40:00',3, 2,15.00, 30.00,  0.00,0.2100, 6.30, 36.30,TRUE, TRUE, 'completed','web','SUV; may need an oversized bay.'),
 (10,'ORD-2024-0010',3,  3,'Klara','Ruzickova','klara.r@example.com','2024-02-25 09:00:00','2024-03-01','2024-03-08','05:00:00','22:30:00',2, 7, 8.00, 56.00,  0.00,0.2100,11.76, 67.76,TRUE, FALSE,'completed','partner',NULL),
 (11,'ORD-2024-0011',1,  1,'Filip','Kucera','filip.kucera@example.com','2024-03-02 11:11:00','2024-03-06','2024-03-20','06:30:00','23:59:00',1,14,15.00,210.00,-30.00,0.2100,37.80,217.80,TRUE, FALSE,'completed','partner',NULL),
 (12,'ORD-2024-0012',NULL,2,'Anna','Urbanova','anna.urbanova@example.com','2024-03-07 13:40:00','2024-03-11','2024-03-13','07:00:00','12:00:00',2, 2,12.00, 24.00,  0.00,0.2100, 5.04, 29.04,FALSE,FALSE,'cart','web','Abandoned cart.'),
 (13,'ORD-2024-0013',2,  3,'Martin','Vesely','martin.vesely@example.com','2024-03-15 08:55:00','2024-03-19','2024-04-02',NULL,NULL,4,14, 8.00,112.00,-12.00,0.2100,21.00,121.00,TRUE, TRUE, 'completed','partner','Group booking; oversized minibus.'),
 (14,'ORD-2024-0014',NULL,1,'Tereza','Blazkova','tereza.b@example.com','2024-03-22 18:05:00','2024-03-25','2024-03-29','10:15:00','14:20:00',2, 4,15.00, 60.00,  0.00,0.2100,12.60, 72.60,TRUE, FALSE,'completed','phone',NULL),
 (15,'ORD-2024-0015',3,  2,'Jakub','Fiala','jakub.fiala@example.com','2024-03-29 15:25:00','2024-04-03','2024-04-10','05:45:00','20:30:00',3, 7,12.00, 84.00,-20.00,0.2100,13.44, 77.44,TRUE, FALSE,'completed','partner',NULL),
 (16,'ORD-2024-0016',1,  1,'Veronika','Kolarova','veronika.k@example.com','2024-04-04 09:30:00','2024-04-08','2024-04-18','06:00:00','21:15:00',2,10,15.00,150.00,  0.00,0.2100,31.50,181.50,TRUE, FALSE,'completed','partner',NULL),
 (17,'ORD-2024-0017',NULL,3,'Roman','Sedlak','roman.sedlak@example.com','2024-04-09 12:00:00','2024-04-13','2024-04-15',NULL,NULL,1, 2, 8.00, 16.00,  0.00,0.2100, 3.36, 19.36,TRUE, FALSE,'completed','web',NULL),
 (18,'ORD-2024-0018',2,  2,'Simona','Markova','simona.m@example.com','2024-04-16 10:45:00','2024-04-22','2024-04-29','08:00:00','10:30:00',2, 7,12.00, 84.00,-12.00,0.2100,15.12, 87.12,TRUE, FALSE,'completed','partner','Please email the invoice to the accounts department; VAT ID is on file.'),
 (19,'ORD-2024-0019',NULL,1,'Pavel','Riha','pavel.riha@example.com','2024-04-21 20:10:00','2024-04-26','2024-04-28','12:30:00','16:00:00',4, 2,15.00, 30.00,  0.00,0.2100, 6.30, 36.30,FALSE,TRUE, 'cancelled','web','No-show.'),
 (20,'ORD-2024-0020',3,  3,'Michaela','Zemanova','michaela.z@example.com','2024-04-27 14:35:00','2024-05-02','2024-05-09','05:15:00','23:00:00',2, 7, 8.00, 56.00,  0.00,0.2100,11.76, 67.76,TRUE, FALSE,'completed','partner',NULL),
 (21,'ORD-2024-0021',1,  1,'Adam','Novotny','adam.novotny@example.com','2024-05-03 08:40:00','2024-05-07','2024-05-21','06:30:00','22:45:00',1,14,15.00,210.00,-30.00,0.2100,37.80,217.80,TRUE, FALSE,'completed','partner',NULL),
 (22,'ORD-2024-0022',NULL,2,'Barbora','Machova','barbora.m@example.com','2024-05-08 11:20:00','2024-05-12','2024-05-14','07:30:00','11:45:00',2, 2,12.00, 24.00,  0.00,0.2100, 5.04, 29.04,TRUE, FALSE,'completed','web',NULL),
 (23,'ORD-2024-0023',2,  3,'Vojtech','Kriz','vojtech.kriz@example.com','2024-05-16 09:05:00','2024-05-20','2024-06-03',NULL,NULL,3,14, 8.00,112.00,-12.00,0.2100,21.00,121.00,TRUE, TRUE, 'completed','partner','Extended stay; oversized 4x4.'),
 (24,'ORD-2024-0024',NULL,1,'Katerina','Dolezalova','katerina.d@example.com','2024-05-23 17:50:00','2024-05-27','2024-05-31','10:00:00','13:30:00',2, 4,15.00, 60.00,  0.00,0.2100,12.60, 72.60,FALSE,FALSE,'cart','phone','Awaiting payment.'),
 (25,'ORD-2024-0025',3,  2,'Lukas','Bartos','lukas.bartos@example.com','2024-05-30 13:15:00','2024-06-04','2024-06-11','05:30:00','21:20:00',4, 7,12.00, 84.00,-20.00,0.2100,13.44, 77.44,TRUE, FALSE,'completed','partner',NULL),
 (26,'ORD-2024-0026',1,  1,'Nikola','Sykorova','nikola.s@example.com','2024-06-05 08:25:00','2024-06-09','2024-06-19','06:00:00','20:50:00',2,10,15.00,150.00,  0.00,0.2100,31.50,181.50,TRUE, FALSE,'completed','partner',NULL),
 (27,'ORD-2024-0027',NULL,3,'Marketa','Simkova','marketa.s@example.com','2024-06-10 15:10:00','2024-06-14','2024-06-16',NULL,NULL,1, 2, 8.00, 16.00,  0.00,0.2100, 3.36, 19.36,TRUE, FALSE,'completed','web',NULL),
 (28,'ORD-2024-0028',2,  2,'Radek','Havel','radek.havel@example.com','2024-06-17 10:35:00','2024-06-23','2024-06-30','08:15:00','09:30:00',2, 7,12.00, 84.00,-12.00,0.2100,15.12, 87.12,TRUE, FALSE,'completed','partner',NULL),
 (29,'ORD-2024-0029',NULL,1,'Denisa','Kadlecova','denisa.k@example.com','2024-06-24 19:00:00','2024-06-28','2024-06-30','12:00:00','15:45:00',3, 2,15.00, 30.00,  0.00,0.2100, 6.30, 36.30,TRUE, TRUE, 'completed','web','Large SUV.'),
 (30,'ORD-2024-0030',3,  3,'Ivan','Pospisil','ivan.pospisil@example.com','2024-06-29 14:00:00','2024-07-03','2024-07-10','05:00:00','22:10:00',2, 7, 8.00, 56.00,  0.00,0.2100,11.76, 67.76,TRUE, FALSE,'completed','partner',NULL);

INSERT INTO order_service (id, order_id, service_id, quantity, line_total) VALUES
 (1,  1, 1, 1, 12.00),
 (2,  1, 3, 2, 17.00),
 (3,  3, 5, 1, 40.00),
 (4,  3, 4, 2, 10.00),
 (5,  6, 2, 1, 25.00),
 (6,  6, 3, 3, 25.50),
 (7,  8, 4, 1,  5.00),
 (8, 11, 5, 1, 40.00),
 (9, 11, 1, 1, 12.00),
 (10,13, 4, 3, 15.00),
 (11,16, 2, 1, 25.00),
 (12,18, 3, 2, 17.00),
 (13,21, 5, 1, 40.00),
 (14,23, 4, 2, 10.00),
 (15,26, 2, 1, 25.00),
 (16,28, 3, 1,  8.50);
