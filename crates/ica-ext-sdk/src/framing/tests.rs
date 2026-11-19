use std::{future::ready, io::Cursor};

use tokio::io::{BufReader, duplex};

use super::*;

#[tokio::test]
async fn exact_max_accepts_crlf_and_oversize_is_rejected_before_write_or_read_to_eof() {
    // 用合法 JSON 字符串填满正文；两侧引号也属于正文。
    let text = "a".repeat(MAX_LINE_BYTES - 2);
    let mut frame = encode_frame(&text).unwrap();
    assert_eq!(frame.len(), MAX_LINE_BYTES + 1);
    frame[MAX_LINE_BYTES] = b'\r';
    frame.push(b'\n');
    let mut reader = FrameReader::new(BufReader::new(Cursor::new(frame)));
    assert_eq!(
        reader.read::<String>().await.unwrap().as_deref(),
        Some(text.as_str())
    );
    assert!(reader.read::<String>().await.unwrap().is_none());

    let extra = text + "a";
    let mut writer = Vec::new();
    assert!(matches!(
        write_frame(&mut writer, &extra).await,
        Err(SdkError::LineTooLong)
    ));
    assert!(writer.is_empty(), "超长动作不能污染 stdout");

    // 输入即便没有行尾，也不能无限累加。
    let mut reader = FrameReader::new(BufReader::new(Cursor::new(vec![b'a'; MAX_LINE_BYTES + 1])));
    assert!(matches!(
        reader.read::<String>().await,
        Err(SdkError::LineTooLong)
    ));
}

#[tokio::test]
async fn cancelled_read_resumes_partial_frame_and_partial_eof_is_not_a_frame() {
    let (mut tx, rx) = duplex(64);
    let mut reader = FrameReader::new(BufReader::new(rx));
    tx.write_all(b"\"incom").await.unwrap();
    tokio::select! {
        biased;
        _ = reader.read::<String>() => panic!("部分帧尚未结束"),
        _ = ready(()) => {},
    }
    tx.write_all(b"plete\"\n\"tail\"").await.unwrap();
    assert_eq!(
        reader.read::<String>().await.unwrap().as_deref(),
        Some("incomplete")
    );
    drop(tx);
    assert!(matches!(
        reader.read::<String>().await,
        Err(SdkError::TruncatedFrame)
    ));
}
