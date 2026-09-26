use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use rextto::{
    cleaner::index_archive,
    database::Database,
    parser::{parse_quality, parse_release},
};
use rusqlite::params;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const MAGNET: &str = "magnet:?xt=urn:btih:0123456789012345678901234567890123456789";

fn temporary_path(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before Unix epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "rextto-bench-{label}-{}-{nonce}",
        std::process::id()
    ))
}

fn bench_parser(c: &mut Criterion) {
    let titles = [
        "The.Example.Show.S01E01.2160p.WEB-DL.DV.HDR.ITA-GROUP",
        "Example.Show.S02E03-E04.1080p.WEBRip.x264.AAC.ITA",
        "A.Movie.2025.2160p.UHD.BluRay.REMUX.HDR10.ITA",
        "Example.Show.S03.1080p.WEB-DL.ITA",
    ];
    let mut group = c.benchmark_group("parser");
    for size in [1_usize, 100, 1_000] {
        group.bench_with_input(BenchmarkId::new("parse_quality", size), &size, |b, size| {
            b.iter(|| {
                let mut score = 0_i64;
                for index in 0..*size {
                    score += parse_quality(titles[index % titles.len()]).score();
                }
                black_box(score)
            });
        });
        group.bench_with_input(BenchmarkId::new("parse_release", size), &size, |b, size| {
            b.iter(|| {
                let mut parsed = 0_usize;
                for index in 0..*size {
                    parsed += usize::from(
                        parse_release(titles[index % titles.len()], MAGNET, "benchmark").is_some(),
                    );
                }
                black_box(parsed)
            });
        });
    }
    group.finish();
}

fn create_database_fixture(series_count: usize, episodes_per_series: usize) -> (Database, PathBuf) {
    let path = temporary_path("database");
    let db = Database::open(&path).expect("open benchmark database");
    let tx = db
        .conn
        .unchecked_transaction()
        .expect("start fixture transaction");
    for series_index in 0..series_count {
        let name = format!("Bench Series {series_index}");
        tx.execute("INSERT INTO series(name) VALUES (?1)", [&name])
            .expect("insert series");
        let series_id = tx.last_insert_rowid();
        let season = 1_i64;
        tx.execute(
            "INSERT INTO series_metadata(series_name,season,episode_count,updated_at) VALUES (?1,?2,?3,datetime('now'))",
            params![name, season, episodes_per_series as i64],
        )
        .expect("insert metadata");
        // Leave every fifth episode absent so gap calculation has meaningful
        // work while retaining enough rows to exercise the joins.
        for episode in 1..=episodes_per_series {
            if episode % 5 == 0 {
                continue;
            }
            tx.execute(
                "INSERT INTO episodes(series_id,season,episode,title,quality_score,downloaded_at) VALUES (?1,1,?2,?3,1000,CASE WHEN ?2 % 3 = 0 THEN datetime('now') ELSE NULL END)",
                params![series_id, episode as i64, format!("{name}.S01E{episode:02}.1080p.WEB-DL")],
            )
            .expect("insert episode");
        }
        for episode in (5..=episodes_per_series).step_by(25) {
            tx.execute(
                "INSERT INTO ignored_episodes(series_name,season,episode) VALUES (?1,1,?2)",
                params![name, episode as i64],
            )
            .expect("insert ignored episode");
        }
    }
    tx.commit().expect("commit fixture transaction");
    (db, path)
}

fn bench_database(c: &mut Criterion) {
    let mut group = c.benchmark_group("database");
    for (label, series, episodes) in [("small", 10, 100), ("medium", 100, 500)] {
        let (db, path) = create_database_fixture(series, episodes);
        group.bench_function(BenchmarkId::new("archive_gaps", label), |b| {
            b.iter(|| black_box(db.archive_gaps().expect("archive gaps")));
        });
        group.bench_function(BenchmarkId::new("episodes_for_series", label), |b| {
            b.iter(|| {
                black_box(
                    db.episodes_for_series("Bench Series 0", &[])
                        .expect("series episodes"),
                )
            });
        });
        drop(db);
        remove_database_files(&path);
    }
    group.finish();
}

fn create_archive_fixture(file_count: usize) -> PathBuf {
    let path = temporary_path("archive");
    fs::create_dir_all(&path).expect("create archive fixture");
    for index in 0..file_count {
        let season = index / 100 + 1;
        let episode = index % 100 + 1;
        let file = path.join(format!(
            "Bench Series S{season:02}E{episode:02}.1080p.WEB-DL.mkv"
        ));
        fs::write(file, []).expect("create archive file");
    }
    path
}

fn bench_archive_index(c: &mut Criterion) {
    let settings = BTreeMap::new();
    let mut group = c.benchmark_group("archive");
    for file_count in [100_usize, 1_000] {
        let path = create_archive_fixture(file_count);
        group.bench_with_input(
            BenchmarkId::new("index_archive", file_count),
            &file_count,
            |b, _| {
                b.iter(|| black_box(index_archive("Bench Series", Path::new(&path), &settings)));
            },
        );
        remove_directory(&path);
    }
    group.finish();
}

fn remove_database_files(path: &Path) {
    let _ = fs::remove_file(path);
    let _ = fs::remove_file(PathBuf::from(format!("{}-wal", path.display())));
    let _ = fs::remove_file(PathBuf::from(format!("{}-shm", path.display())));
}

fn remove_directory(path: &Path) {
    let _ = fs::remove_dir_all(path);
}

criterion_group!(
    name = benches;
    config = Criterion::default()
        .warm_up_time(Duration::from_secs(1))
        .measurement_time(Duration::from_secs(2))
        .sample_size(30);
    targets = bench_parser, bench_database, bench_archive_index
);
criterion_main!(benches);
