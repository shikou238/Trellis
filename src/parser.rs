
use std::fmt;

#[derive(Debug, Clone, PartialEq)]
enum TokenKind {
    LParen, // `(`
    RParen, // `)`
    LBrace, // `{`
    RBrace, // `}`

    Plus, // `+`
    Minus, // `-`
    Star, // `*`
    Slash, // `/`
    Bang, // `!`
    EqEq, // `==`
    NotEq, // `!=`
    Less, // `<`
    LessEq, // `<=`
    Greater, // `>`
    GreaterEq, // `>=`

    Integer(i64),
    Float(f64),
    String(String),
    Symbol(String),

    Eof,
}

/// ソース上の位置（char インデックス）。エラー表示とソースへの書き戻しに使う
#[derive(Debug, Clone, PartialEq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

#[derive(Debug, Clone, PartialEq)]
struct Token {
    kind: TokenKind,
    span: Span,
}

#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub span: Span,
}

impl std::error::Error for ParseError {}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} at byte {}..{}",
            self.message, self.span.start, self.span.end
        )
    }
}

pub type ParseResult<T> = Result<T, ParseError>;

#[derive(Debug, Clone)]
pub struct AstNode {
    pub kind: AstKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum AstKind {
    Form {
        head: String,
        args: Vec<AstNode>,
    },

    Expr(Expr),

    Integer(i64),
    Float(f64),
    String(String),
    Symbol(String),
}

#[derive(Debug, Clone)]
pub enum Expr {
    Integer(i64),
    Float(f64),
    Symbol(String),

    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },

    Binary {
        op: BinaryOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },

    Call {
        callee: String,
        args: Vec<Expr>,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum UnaryOp {
    Neg,
    Not,
}

#[derive(Debug, Clone, Copy)]
pub enum BinaryOp {
    // 比較（結果は 1 か 0）。+ - より弱く結合する
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,

    Add,
    Sub,
    Mul,
    Div,
}

struct Lexer<'a> {
    source: &'a str,
    chars: Vec<char>,
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            chars: source.chars().collect(),
            pos: 0,
        }
    }

    fn tokenize(mut self) -> ParseResult<Vec<Token>> {
        let mut tokens = Vec::new();

        loop {
            self.skip_whitespace_and_comments();

            let start = self.pos;
            let Some(ch) = self.peek() else {
                tokens.push(Token {
                    kind: TokenKind::Eof,
                    span: Span {
                        start: self.pos,
                        end: self.pos,
                    },
                });
                break;
            };

            let kind = match ch {
                '(' => {
                    self.advance();
                    TokenKind::LParen
                }
                ')' => {
                    self.advance();
                    TokenKind::RParen
                }
                '{' => {
                    self.advance();
                    TokenKind::LBrace
                }
                '}' => {
                    self.advance();
                    TokenKind::RBrace
                }
                '+' => {
                    self.advance();
                    TokenKind::Plus
                }
                '-' => {
                    self.advance();
                    TokenKind::Minus
                }
                '*' => {
                    self.advance();
                    TokenKind::Star
                }
                '/' => {
                    self.advance();
                    TokenKind::Slash
                }
                '!' => {
                    self.advance();
                    if self.eat('=') { TokenKind::NotEq } else { TokenKind::Bang }
                }
                '=' if self.peek_next() == Some('=') => {
                    self.advance();
                    self.advance();
                    TokenKind::EqEq
                }
                '<' => {
                    self.advance();
                    if self.eat('=') { TokenKind::LessEq } else { TokenKind::Less }
                }
                '>' => {
                    self.advance();
                    if self.eat('=') { TokenKind::GreaterEq } else { TokenKind::Greater }
                }
                '"' => self.read_string().map_err(|m| self.error_from(start, m))?,
                c if c.is_ascii_digit() => self.read_number().map_err(|m| self.error_from(start, m))?,
                c if is_ident_start(c) => self.read_symbol(),
                _ => {
                    self.advance();
                    return Err(self.error_from(start, format!("unexpected character {:?}", ch)));
                }
            };

            tokens.push(Token {
                kind,
                span: Span {
                    start,
                    end: self.pos,
                },
            });
        }

        Ok(tokens)
    }

    fn error_from(&self, start: usize, message: String) -> ParseError {
        ParseError {
            message,
            span: Span {
                start,
                end: self.pos,
            },
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    /// 次の文字が `expected` なら読み進めて true
    fn eat(&mut self, expected: char) -> bool {
        if self.peek() == Some(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn peek_next(&self) -> Option<char> {
        self.chars.get(self.pos + 1).copied()
    }

    fn advance(&mut self) -> Option<char> {
        let ch = self.peek();
        if ch.is_some() {
            self.pos += 1;
        }
        ch
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            while matches!(self.peek(), Some(ch) if ch.is_whitespace()) {
                self.advance();
            }

            let is_line_comment = self.peek() == Some(';')
                && self.peek_next() == Some(';');

            if is_line_comment {
                while let Some(ch) = self.peek() {
                    self.advance();
                    if ch == '\n' {
                        break;
                    }
                }
                continue;
            }

            break;
        }
    }

    fn read_string(&mut self) -> Result<TokenKind, String> {
        self.advance();

        let mut value = String::new();

        while let Some(ch) = self.advance() {
            match ch {
                '"' => return Ok(TokenKind::String(value)),

                '\\' => {
                    let Some(escaped) = self.advance() else {
                        return Err("unterminated escape sequence".to_string());
                    };

                    let decoded = match escaped {
                        'n' => '\n',
                        't' => '\t',
                        '"' => '"',
                        '\\' => '\\',
                        other => {
                            return Err(format!("unknown escape sequence \\{}", other));
                        }
                    };

                    value.push(decoded);
                }

                other => value.push(other),
            }
        }

        Err("unterminated string literal".to_string())
    }

    fn read_number(&mut self) -> Result<TokenKind, String> {
        let start = self.pos;
        let mut saw_dot = false;

        while let Some(ch) = self.peek() {
            match ch {
                '0'..='9' => {
                    self.advance();
                }

                '.' if !saw_dot => {
                    saw_dot = true;
                    self.advance();
                }

                _ => break,
            }
        }

        let text: String = self.chars[start..self.pos].iter().collect();

        if saw_dot {
            let value = text
                .parse::<f64>()
                .map_err(|_| format!("invalid float literal: {}", text))?;

            Ok(TokenKind::Float(value))
        } else {
            let value = text
                .parse::<i64>()
                .map_err(|_| format!("invalid integer literal: {}", text))?;

            Ok(TokenKind::Integer(value))
        }
    }

    fn read_symbol(&mut self) -> TokenKind {
        let start = self.pos;

        self.advance();

        while matches!(self.peek(), Some(ch) if is_ident_continue(ch)) {
            self.advance();
        }

        let name: String = self.chars[start..self.pos].iter().collect();

        TokenKind::Symbol(name)
    }
}

fn is_ident_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == '_' || ch == '?'
}

fn is_ident_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric()
        || ch == '_'
        || ch == '-'
        || ch == '?'
        || ch == '!'
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    fn parse_program(&mut self) -> ParseResult<Vec<AstNode>> {
        let mut forms = Vec::new();

        while !self.is_eof() {
            forms.push(self.parse_node()?);
        }

        Ok(forms)
    }

    fn parse_node(&mut self) -> ParseResult<AstNode> {
        let start = self.current().span.start;

        let kind = match &self.current().kind {
            TokenKind::LParen => self.parse_form()?,
            TokenKind::LBrace => AstKind::Expr(self.parse_braced_expression()?),
            // `-1` のように数値に密着した `-` は負のリテラルとして扱う
            TokenKind::Minus if self.next_is_adjacent_number() => {
                self.advance();
                match self.parse_node()?.kind {
                    AstKind::Integer(value) => AstKind::Integer(-value),
                    AstKind::Float(value) => AstKind::Float(-value),
                    _ => unreachable!(),
                }
            }
            TokenKind::Integer(value) => {
                let value = *value;
                self.advance();
                AstKind::Integer(value)
            }
            TokenKind::Float(value) => {
                let value = *value;
                self.advance();
                AstKind::Float(value)
            }
            TokenKind::String(value) => {
                let value = value.clone();
                self.advance();
                AstKind::String(value)
            }
            TokenKind::Symbol(value) => {
                let value = value.clone();
                self.advance();
                AstKind::Symbol(value)
            }
            _ => return Err(self.error_here("expected form, expression, literal, or symbol")),
        };

        Ok(AstNode {
            kind,
            span: Span {
                start,
                end: self.previous_end(),
            },
        })
    }

    fn parse_form(&mut self) -> ParseResult<AstKind> {
        self.expect_lparen()?;

        let head = self.expect_symbol()?;
        let mut args = Vec::new();

        while !matches!(self.current().kind, TokenKind::RParen | TokenKind::Eof) {
            args.push(self.parse_node()?);
        }

        self.expect_rparen()?;

        Ok(AstKind::Form { head, args })
    }

    fn next_is_adjacent_number(&self) -> bool {
        let Some(next) = self.tokens.get(self.pos + 1) else {
            return false;
        };
        matches!(next.kind, TokenKind::Integer(_) | TokenKind::Float(_))
            && next.span.start == self.current().span.end
    }

    fn previous_end(&self) -> usize {
        self.tokens[self.pos.saturating_sub(1)].span.end
    }

    fn parse_braced_expression(&mut self) -> ParseResult<Expr> {
        self.expect_lbrace()?;

        let expr = self.parse_expr_bp(0)?;

        self.expect_rbrace()?;

        Ok(expr)
    }

    fn parse_expr_bp(&mut self, min_bp: u8) -> ParseResult<Expr> {

        //caution : 累乗には対応していない
        let mut left = self.parse_prefix_expr()?;

        loop {
            let Some((op, left_bp, right_bp)) = self.current_binary_op() else {
                break;
            };
            if left_bp < min_bp {
                break;
            }

            self.advance();

            let right = self.parse_expr_bp(right_bp)?;

            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }

        Ok(left)
    }

    fn parse_prefix_expr(&mut self) -> ParseResult<Expr> {
        match &self.current().kind {
            TokenKind::Integer(value) => {
                let value = *value;
                self.advance();
                Ok(Expr::Integer(value))
            }

            TokenKind::Float(value) => {
                let value = *value;
                self.advance();
                Ok(Expr::Float(value))
            }

            TokenKind::Symbol(name) => {
                let name = name.clone();
                self.advance();

                // 旧い書き方 `f(a, b)` には新しい書き方を案内する
                if matches!(self.current().kind, TokenKind::LParen) {
                    return Err(self.error_here(format!(
                        "function calls are written in S-expression form: `({} a b)`, not `{}(a, b)`",
                        name, name
                    )));
                }
                Ok(Expr::Symbol(name))
            }

            TokenKind::Minus => {
                self.advance();

                let expr = self.parse_expr_bp(100)?;

                Ok(Expr::Unary {
                    op: UnaryOp::Neg,
                    expr: Box::new(expr),
                })
            }

            TokenKind::Bang => {
                self.advance();

                let expr = self.parse_expr_bp(100)?;

                Ok(Expr::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                })
            }
            // `{}` の中の `( )` は S 式の関数呼び出し: (f 引数...)
            TokenKind::LParen => self.parse_expr_call(),

            // 結合の順番を変えたいときは `{ }` を入れ子にする: {{a + b} * 2}
            TokenKind::LBrace => self.parse_braced_expression(),

            _ => Err(self.error_here("expected expression")),
        }
    }

    /// (f 引数...)。引数は 数値 / 名前 / {中置式} / (関数呼び出し) のどれか。中置の演算は {} で囲む
    fn parse_expr_call(&mut self) -> ParseResult<Expr> {
        self.expect_lparen()?;

        const GROUPING: &str = "`( )` is a function call `(f a b)`; to group an expression use `{ }`, e.g. `{{a + b} * 2}`";
        let callee = match &self.current().kind {
            TokenKind::Symbol(name) => {
                let name = name.clone();
                self.advance();
                name
            }
            _ => return Err(self.error_here(GROUPING)),
        };
        // `(a + b)` のように関数名の直後に演算子が来たら、結合のためのかっこと判断して案内する
        // ただし `(abs -2)` の `-2` は負のリテラルなので除く
        let negative_literal = self.current().kind == TokenKind::Minus && self.next_is_adjacent_number();
        if self.current_binary_op().is_some() && !negative_literal {
            return Err(self.error_here(GROUPING));
        }

        let mut args = Vec::new();
        while !matches!(self.current().kind, TokenKind::RParen | TokenKind::Eof) {
            args.push(self.parse_call_arg()?);
        }

        self.expect_rparen()?;

        Ok(Expr::Call { callee, args })
    }

    fn parse_call_arg(&mut self) -> ParseResult<Expr> {
        match &self.current().kind {
            // 名前の直後に次の引数の `(` が来てもよいので、parse_prefix_expr を通さずに読む
            TokenKind::Symbol(name) => {
                let name = name.clone();
                self.advance();
                Ok(Expr::Symbol(name))
            }
            TokenKind::Integer(_) | TokenKind::Float(_) | TokenKind::LParen | TokenKind::LBrace => {
                self.parse_prefix_expr()
            }
            // `-1` のように数値に密着した `-` は負のリテラル
            TokenKind::Minus if self.next_is_adjacent_number() => {
                self.advance();
                Ok(Expr::Unary { op: UnaryOp::Neg, expr: Box::new(self.parse_prefix_expr()?) })
            }
            _ => Err(self.error_here("expected a number, name, {expression}, or (call) as an argument; wrap operators in {}")),
        }
    }

    fn current_binary_op(&self) -> Option<(BinaryOp, u8, u8)> {
        match self.current().kind {
            TokenKind::EqEq => Some((BinaryOp::Eq, 1, 2)),
            TokenKind::NotEq => Some((BinaryOp::Ne, 1, 2)),
            TokenKind::Less => Some((BinaryOp::Lt, 1, 2)),
            TokenKind::LessEq => Some((BinaryOp::Le, 1, 2)),
            TokenKind::Greater => Some((BinaryOp::Gt, 1, 2)),
            TokenKind::GreaterEq => Some((BinaryOp::Ge, 1, 2)),
            TokenKind::Plus => Some((BinaryOp::Add, 3, 4)),
            TokenKind::Minus => Some((BinaryOp::Sub, 3, 4)),
            TokenKind::Star => Some((BinaryOp::Mul, 5, 6)),
            TokenKind::Slash => Some((BinaryOp::Div, 5, 6)),
            _ => None,
        }
    }

    fn current(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn is_eof(&self) -> bool {
        self.current().kind == TokenKind::Eof
    }

    fn advance(&mut self) {
        if !self.is_eof() {
            self.pos += 1;
        }
    }

    fn expect_lparen(&mut self) -> ParseResult<()> {
        if matches!(self.current().kind, TokenKind::LParen) {
            self.advance();
            Ok(())
        } else {
            Err(self.error_here("expected `(`"))
        }
    }

    fn expect_rparen(&mut self) -> ParseResult<()> {
        if matches!(self.current().kind, TokenKind::RParen) {
            self.advance();
            Ok(())
        } else {
            Err(self.error_here("expected `)`"))
        }
    }

    fn expect_lbrace(&mut self) -> ParseResult<()> {
        if matches!(self.current().kind, TokenKind::LBrace) {
            self.advance();
            Ok(())
        } else {
            Err(self.error_here("expected `{`"))
        }
    }

    fn expect_rbrace(&mut self) -> ParseResult<()> {
        if matches!(self.current().kind, TokenKind::RBrace) {
            self.advance();
            Ok(())
        } else {
            Err(self.error_here("expected `}`"))
        }
    }

    fn expect_symbol(&mut self) -> ParseResult<String> {
        match &self.current().kind {
            TokenKind::Symbol(name) => {
                let name = name.clone();
                self.advance();
                Ok(name)
            }
            _ => Err(self.error_here("expected symbol")),
        }
    }

    fn error_here(&self, message: impl Into<String>) -> ParseError {
        ParseError {
            message: message.into(),
            span: self.current().span.clone(),
        }
    }
}

/// ソース全体をパースしてトップレベルのノード列を返す
pub fn parse(source: &str) -> ParseResult<Vec<AstNode>> {
    let tokens = Lexer::new(source).tokenize()?;
    Parser::new(tokens).parse_program()
}

/// char インデックスを「行:列」（1始まり）に変換する
pub fn line_col(source: &str, index: usize) -> (usize, usize) {
    let mut line = 1;
    let mut col = 1;
    for ch in source.chars().take(index) {
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}
