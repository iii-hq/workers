# Writer Agent Task
Write orders to the database and emit change signals.

Task: Insert 5 orders, 2 seconds apart.
Each order: id (auto), writer: X, amount: 100.0-150.0 randomized, created_at: now

Mark done in rctest7_a7b9_writers table.
Emit change signal after each insert.

State scope: rctest7_a7b9_run
Change signal key: rctest7_a7b9_last_change