-- Native change capture (`capture: native`) reads the binlog as a replica.
-- REPLICATION CLIENT: SHOW BINARY LOG STATUS (position snapshot + the
-- registration-time privilege probe). REPLICATION SLAVE: COM_BINLOG_DUMP.
GRANT REPLICATION SLAVE, REPLICATION CLIENT ON *.* TO 'iii'@'%';
FLUSH PRIVILEGES;
