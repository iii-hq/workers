export type DriverKey = 'sqlite_db' | 'pg_db' | 'mysql_db';

export const DRIVER_KEYS: readonly DriverKey[] = ['sqlite_db', 'pg_db', 'mysql_db'] as const;

export interface Dialect {
  /** Returns the parameter placeholder for the i-th (1-indexed) bound value. */
  placeholder(i: number): string;
  /** DDL fragment for the auto-increment primary-key id column. */
  idColumnDDL(): string;
}

export const dialects: Record<DriverKey, Dialect> = {
  sqlite_db: {
    placeholder: () => '?',
    idColumnDDL: () => 'INTEGER PRIMARY KEY AUTOINCREMENT',
  },
  pg_db: {
    placeholder: (i: number) => `$${i}`,
    idColumnDDL: () => 'BIGSERIAL PRIMARY KEY',
  },
  mysql_db: {
    placeholder: () => '?',
    idColumnDDL: () => 'BIGINT AUTO_INCREMENT PRIMARY KEY',
  },
};
