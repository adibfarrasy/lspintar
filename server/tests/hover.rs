use std::env;
use std::sync::Arc;

use lspintar_server::{Repository, server::Backend};
use pretty_assertions::assert_eq;
use tower_lsp::{
    LanguageServer, LspService,
    lsp_types::{
        Hover, HoverContents, HoverParams, InitializeParams, InitializedParams, MarkupContent,
        MarkupKind, Position, TextDocumentIdentifier, TextDocumentPositionParams, Url,
        WorkDoneProgressParams,
    },
};
use uuid::Uuid;

struct TestServer {
    backend: Backend,
}

impl TestServer {
    async fn new(fixture: &str) -> Self {
        let db_name = Uuid::new_v4();
        let db_dir = format!("file:{}?mode=memory&cache=shared", db_name);
        let repo = Arc::new(Repository::new(&db_dir).await.unwrap());
        let (service, _socket) = LspService::new(|client| Backend::new(client, repo.clone()));
        let backend = Backend::new(service.inner().client.clone(), repo.clone());

        let root = env::current_dir().expect("cannot get current dir");

        let mut init_params = InitializeParams::default();
        init_params.root_uri = Some(
            Url::from_file_path(root.join("tests/fixtures").join(fixture))
                .expect("cannot parse root URI"),
        );
        backend.initialize(init_params).await.unwrap();
        backend.initialized(InitializedParams {}).await;

        Self { backend }
    }
}

#[tokio::test]
async fn test_hover_project_symbol() {
    let server = TestServer::new("groovy-gradle-multi").await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = HoverParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/groovy-gradle-multi/core/src/main/groovy/com/example/core/DataProcessor.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(4, 11),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
    };

    let result = server.backend.hover(params).await.unwrap();
    assert!(result.is_some());

    let hover = Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: "```groovy\npackage com.example.core\n\ninterface DataProcessor\n```"
                .to_string(),
        }),
        range: None,
    };

    assert_eq!(result.unwrap(), hover);
}

#[tokio::test]
async fn test_hover_external_symbol() {
    let server = TestServer::new("polyglot-spring").await;

    let root = env::current_dir().expect("cannot get current dir");

    let params = HoverParams {
        text_document_position_params: TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: Url::from_file_path(root.join("tests/fixtures/polyglot-spring/src/main/groovy/com/example/demo/Controller.groovy"))
                    .expect("cannot parse root URI"),
            },
            position: Position::new(24, 24),
        },
        work_done_progress_params: WorkDoneProgressParams::default(),
    };

    let result = server.backend.hover(params).await.unwrap();
    assert!(result.is_some());

    let hover = Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: "```java\npackage org.apache.commons.lang3\n\npublic class StringUtils\n\n---\n```\nOperations on {@link java.lang.String} that are\n{@code null} safe.\n\n<ul>\n<li><b>IsEmpty/IsBlank</b>\n- checks if a String contains text</li>\n<li><b>Trim/Strip</b>\n- removes leading and trailing whitespace</li>\n<li><b>Equals/Compare</b>\n- compares two strings in a null-safe manner</li>\n<li><b>startsWith</b>\n- check if a String starts with a prefix in a null-safe manner</li>\n<li><b>endsWith</b>\n- check if a String ends with a suffix in a null-safe manner</li>\n<li><b>IndexOf/LastIndexOf/Contains</b>\n- null-safe index-of checks\n<li><b>IndexOfAny/LastIndexOfAny/IndexOfAnyBut/LastIndexOfAnyBut</b>\n- index-of any of a set of Strings</li>\n<li><b>ContainsOnly/ContainsNone/ContainsAny</b>\n- checks if String contains only/none/any of these characters</li>\n<li><b>Substring/Left/Right/Mid</b>\n- null-safe substring extractions</li>\n<li><b>SubstringBefore/SubstringAfter/SubstringBetween</b>\n- substring extraction relative to other strings</li>\n<li><b>Split/Join</b>\n- splits a String into an array of substrings and vice versa</li>\n<li><b>Remove/Delete</b>\n- removes part of a String</li>\n<li><b>Replace/Overlay</b>\n- Searches a String and replaces one String with another</li>\n<li><b>Chomp/Chop</b>\n- removes the last part of a String</li>\n<li><b>AppendIfMissing</b>\n- appends a suffix to the end of the String if not present</li>\n<li><b>PrependIfMissing</b>\n- prepends a prefix to the start of the String if not present</li>\n<li><b>LeftPad/RightPad/Center/Repeat</b>\n- pads a String</li>\n<li><b>UpperCase/LowerCase/SwapCase/Capitalize/Uncapitalize</b>\n- changes the case of a String</li>\n<li><b>CountMatches</b>\n- counts the number of occurrences of one String in another</li>\n<li><b>IsAlpha/IsNumeric/IsWhitespace/IsAsciiPrintable</b>\n- checks the characters in a String</li>\n<li><b>DefaultString</b>\n- protects against a null input String</li>\n<li><b>Rotate</b>\n- rotate (circular shift) a String</li>\n<li><b>Reverse/ReverseDelimited</b>\n- reverses a String</li>\n<li><b>Abbreviate</b>\n- abbreviates a string using ellipses or another given String</li>\n<li><b>Difference</b>\n- compares Strings and reports on their differences</li>\n<li><b>LevenshteinDistance</b>\n- the number of changes needed to change one String into another</li>\n</ul>\n\n<p>The {@link StringUtils} class defines certain words related to\nString handling.</p>\n\n<ul>\n<li>null - {@code null}</li>\n<li>empty - a zero-length string ({@code \"\"})</li>\n<li>space - the space character ({@code ' '}, char 32)</li>\n<li>whitespace - the characters defined by {@link Character#isWhitespace(char)}</li>\n<li>trim - the characters &lt;= 32 as in {@link String#trim()}</li>\n</ul>\n\n<p>{@link StringUtils} handles {@code null} input Strings quietly.\nThat is to say that a {@code null} input will return {@code null}.\nWhere a {@code boolean} or {@code int} is being returned\ndetails vary by method.</p>\n\n<p>A side effect of the {@code null} handling is that a\n{@link NullPointerException} should be considered a bug in\n{@link StringUtils}.</p>\n\n<p>Methods in this class include sample code in their Javadoc comments to explain their operation.\nThe symbol {@code *} is used to indicate any input including {@code null}.</p>\n\n<p>#ThreadSafe#</p>\n@see String\n@since 1.0"
                .to_string(),
        }),
        range: None,
    };

    assert_eq!(result.unwrap(), hover);
}
