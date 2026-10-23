//! Paging a long column: `offset` on `kanban::board::get` (per column) and
//! `kanban::ticket::list`, and the `total` each reports. The native app reads
//! the first rows of every column and pages the rest as the person scrolls.

use iii_kanban::board::{
    BoardContext, CreateTicketInput, TicketFilter, create_ticket, list_tickets, move_ticket, page,
};
use iii_kanban::config::{Column, KanbanConfig};
use iii_kanban::functions::{BoardGetInput, TicketListInput, board_view, ticket_list_view};
use iii_kanban::store::Board;
use serde_json::{Value, json};

fn context() -> BoardContext {
    BoardContext {
        prefix: "KAN".into(),
        default_status: "backlog".into(),
        done_status: "done".into(),
        canceled_status: None,
        columns: [("backlog", "Backlog"), ("todo", "To do"), ("done", "Done")]
            .into_iter()
            .map(|(id, label)| Column {
                id: id.into(),
                label: label.into(),
            })
            .collect(),
        priorities: ["low", "medium", "high", "urgent"]
            .into_iter()
            .map(String::from)
            .collect(),
    }
}

fn config() -> KanbanConfig {
    KanbanConfig {
        columns: context().columns,
        default_status: "backlog".into(),
        ..KanbanConfig::default()
    }
}

fn at(second: u32) -> String {
    format!("2026-01-01T00:{:02}:{:02}.000Z", second / 60, second % 60)
}

/// A board with `done` tickets in Done (KAN-1 … KAN-n, in that order) and two in To do.
fn board(done: u32) -> Board {
    let mut board = Board::empty();
    let mut second = 0;
    for title in ["Todo A", "Todo B"] {
        second += 1;
        let input = CreateTicketInput {
            title: title.into(),
            status: Some("todo".into()),
            ..CreateTicketInput::default()
        };
        create_ticket(&mut board, &input, &context(), &at(second)).unwrap();
    }
    for n in 0..done {
        second += 1;
        let input = CreateTicketInput {
            title: format!("Done {n}"),
            status: Some("done".into()),
            ..CreateTicketInput::default()
        };
        create_ticket(&mut board, &input, &context(), &at(second)).unwrap();
    }
    board
}

fn titles(tickets: &[impl serde::Serialize]) -> Vec<String> {
    tickets
        .iter()
        .map(|row| {
            serde_json::to_value(row).unwrap()["title"]
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect()
}

fn list(input: Value) -> TicketListInput {
    serde_json::from_value(input).unwrap()
}

fn board_input(input: Value) -> BoardGetInput {
    serde_json::from_value(input).unwrap()
}

#[test]
fn page_skips_then_limits_and_an_offset_past_the_end_is_empty() {
    let rows: Vec<u32> = (0..10).collect();
    assert_eq!(page(rows.clone(), None, None), rows);
    assert_eq!(page(rows.clone(), Some(3), Some(4)), vec![3, 4, 5, 6]);
    assert_eq!(page(rows.clone(), Some(8), Some(4)), vec![8, 9]);
    assert_eq!(page(rows.clone(), Some(10), Some(4)), Vec::<u32>::new());
    assert_eq!(page(rows.clone(), Some(99), None), Vec::<u32>::new());
    assert_eq!(
        page(rows.clone(), Some(7), Some(0)),
        vec![7, 8, 9],
        "limit 0 = the rest"
    );
}

#[test]
fn ticket_list_pages_cover_a_long_column_once_in_board_order() {
    let board = board(23);
    let mut seen = Vec::new();
    let mut offset = 0;
    loop {
        let page = ticket_list_view(
            &board,
            &list(json!({ "status": "done", "offset": offset, "limit": 10 })),
        );
        assert_eq!(
            page.total, 23,
            "total counts every match, whatever the page"
        );
        assert_eq!(page.count as usize, page.tickets.len());
        if page.tickets.is_empty() {
            break;
        }
        offset += page.tickets.len();
        seen.extend(titles(&page.tickets));
    }
    let expected: Vec<String> = (0..23).map(|n| format!("Done {n}")).collect();
    assert_eq!(seen, expected, "no gap, no repeat, the worker's order");
}

#[test]
fn ticket_list_without_offset_reads_as_before() {
    let board = board(5);
    let all = ticket_list_view(&board, &list(json!({ "status": "done" })));
    assert_eq!((all.count, all.total), (5, 5));
    let first = ticket_list_view(&board, &list(json!({ "status": "done", "limit": 2 })));
    assert_eq!(titles(&first.tickets), ["Done 0", "Done 1"]);
    assert_eq!((first.count, first.total), (2, 5));
    let past = ticket_list_view(&board, &list(json!({ "status": "done", "offset": 50 })));
    assert_eq!((past.count, past.total), (0, 5));
}

#[test]
fn board_get_reports_each_columns_total_and_pages_every_column() {
    let board = board(12);
    let config = config();

    let whole = board_view(&board, &config, &BoardGetInput::default());
    let totals: Vec<(String, u64, usize)> = whole
        .columns
        .iter()
        .map(|column| (column.id.clone(), column.total, column.tickets.len()))
        .collect();
    assert_eq!(
        totals,
        [
            ("backlog".to_string(), 0, 0),
            ("todo".to_string(), 2, 2),
            ("done".to_string(), 12, 12)
        ]
    );

    let first = board_view(&board, &config, &board_input(json!({ "limit": 5 })));
    let done = &first.columns[2];
    assert_eq!(
        (done.total, done.tickets.len()),
        (12, 5),
        "total is before the limit"
    );
    assert_eq!(first.columns[1].tickets.len(), 2);

    let second = board_view(
        &board,
        &config,
        &board_input(json!({ "offset": 5, "limit": 5, "statuses": ["done"] })),
    );
    assert_eq!(second.columns.len(), 1);
    assert_eq!(
        titles(&second.columns[0].tickets),
        ["Done 5", "Done 6", "Done 7", "Done 8", "Done 9"]
    );
    assert_eq!(second.columns[0].total, 12);

    let filtered = board_view(
        &board,
        &config,
        &board_input(json!({ "updated_since": at(10), "limit": 1 })),
    );
    assert_eq!(
        filtered.columns[2].total, 4,
        "the total counts what passes the filters (Done 8 … Done 11)"
    );
    assert_eq!(filtered.ticket_count, 14, "ticket_count stays board-wide");
}

#[test]
fn a_move_renumbers_so_the_next_page_starts_where_the_rows_end() {
    let mut board = board(6);
    // A ticket of the first page moves to the end of the column: the page boundary shifts by one.
    let first = list_tickets(
        &board,
        &TicketFilter {
            status: Some("done".into()),
            limit: Some(3),
            ..TicketFilter::default()
        },
    );
    let moving = first[0].id.clone();
    move_ticket(
        &mut board,
        &moving,
        "done",
        None,
        None,
        &context(),
        &at(100),
    )
    .unwrap();
    let keys = |offset: usize| -> Vec<String> {
        list_tickets(
            &board,
            &TicketFilter {
                status: Some("done".into()),
                offset: Some(offset),
                limit: Some(3),
                ..TicketFilter::default()
            },
        )
        .into_iter()
        .map(|ticket| ticket.title.clone())
        .collect()
    };
    assert_eq!(keys(0), ["Done 1", "Done 2", "Done 3"]);
    assert_eq!(keys(3), ["Done 4", "Done 5", "Done 0"]);
}

#[test]
fn the_schemas_carry_offset_and_total() {
    let schema = |value: schemars::schema::RootSchema| serde_json::to_value(value).unwrap();
    let board_in = schema(schemars::schema_for!(BoardGetInput));
    let list_in = schema(schemars::schema_for!(TicketListInput));
    for input in [&board_in, &list_in] {
        assert!(input["properties"]["offset"].is_object());
        assert!(input["properties"]["limit"].is_object());
    }
    let board_out = schema(schemars::schema_for!(iii_kanban::functions::BoardView));
    let column = &board_out["definitions"]["BoardColumnView"];
    assert!(
        column["required"]
            .as_array()
            .unwrap()
            .contains(&json!("total"))
    );
    let list_out = schema(schemars::schema_for!(iii_kanban::functions::TicketList));
    assert!(
        list_out["required"]
            .as_array()
            .unwrap()
            .contains(&json!("total"))
    );
}
