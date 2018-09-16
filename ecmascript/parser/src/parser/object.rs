//! Parser for object literal.

use super::*;

#[parser]
impl<'a, I: Input> Parser<'a, I> {
    /// Parse a object literal or object pattern.
    pub(super) fn parse_object<T>(&mut self) -> PResult<'a, T>
    where
        Self: ParseObject<'a, T>,
    {
        let start = cur_pos!();
        assert_and_bump!('{');

        let mut props = vec![];

        let mut first = true;
        while !eat!('}') {
            // Handle comma
            if first {
                first = false;
            } else {
                expect!(',');
                if eat!('}') {
                    break;
                }
            }

            let prop = self.parse_object_prop()?;
            props.push(prop);
        }

        Ok(Self::make_object(span!(start), props))
    }

    /// spec: 'PropertyName'
    pub(super) fn parse_prop_name(&mut self) -> PResult<'a, PropName> {
        let start = cur_pos!();

        let v = match *cur!()? {
            Token::Str { .. } => match bump!() {
                Token::Str { value, has_escape } => PropName::Str(Str {
                    span: span!(start),
                    value,
                    has_escape,
                }),
                _ => unreachable!(),
            },
            Num(_) => match bump!() {
                Num(value) => PropName::Num(Number {
                    span: span!(start),
                    value,
                }),
                _ => unreachable!(),
            },
            Word(..) => match bump!() {
                Word(w) => PropName::Ident(Ident {
                    span: span!(start),
                    sym: w.into(),
                }),
                _ => unreachable!(),
            },
            LBracket => {
                bump!();
                let expr = self
                    .include_in_expr(true)
                    .parse_assignment_expr()
                    .map(PropName::Computed)?;
                expect!(']');
                expr
            }
            _ => unexpected!(),
        };

        Ok(v)
    }
}

#[parser]
impl<'a, I: Input> ParseObject<'a, (Box<Expr>)> for Parser<'a, I> {
    type Prop = Prop;

    fn make_object(span: Span, props: Vec<Self::Prop>) -> Box<Expr> {
        box Expr::Object(ObjectLit { span, props })
    }

    /// spec: 'PropertyDefinition'
    fn parse_object_prop(&mut self) -> PResult<'a, Self::Prop> {
        let start = cur_pos!();
        // Parse as 'MethodDefinition'

        if eat!('*') {
            let span_of_gen = span!(start);

            let name = self.parse_prop_name()?;
            return self
                .parse_fn_args_body(
                    start,
                    Parser::parse_unique_formal_params,
                    None,
                    Some(span_of_gen),
                ).map(|function| {
                    Prop::Method(MethodProp {
                        key: name,
                        function,
                    })
                });
        }

        let key = self.parse_prop_name()?;
        //
        // {[computed()]: a,}
        // { 'a': a, }
        // { 0: 1, }
        // { a: expr, }
        if eat!(':') {
            let value = self.include_in_expr(true).parse_assignment_expr()?;
            return Ok(Prop::KeyValue(KeyValueProp { key, value }));
        }

        // Handle `a(){}` (and async(){} / get(){} / set(){})
        if is!('(') {
            return self
                .parse_fn_args_body(start, Parser::parse_unique_formal_params, None, None)
                .map(|function| Prop::Method(MethodProp { key, function }));
        }

        let ident = match key {
            PropName::Ident(ident) => ident,
            _ => unexpected!(),
        };

        // `ident` from parse_prop_name is parsed as 'IdentifierName'
        // It means we should check for invalid expressions like { for, }
        if is_one_of!('=', ',', '}') {
            let is_reserved_word = { self.ctx().is_reserved_word(&ident.sym) };
            if is_reserved_word {
                syntax_error!(ident.span, SyntaxError::ReservedWordInObjShorthandOrPat);
            }

            if eat!('=') {
                let value = self.include_in_expr(true).parse_assignment_expr()?;
                return Ok(Prop::Assign(AssignProp { key: ident, value }));
            }
            return Ok(ident.into());
        }

        // get a(){}
        // set a(v){}
        // async a(){}

        match ident.sym {
            js_word!("get") | js_word!("set") | js_word!("async") => {
                let key = self.parse_prop_name()?;

                return match ident.sym {
                    js_word!("get") => self
                        .parse_fn_args_body(start, |_| Ok(vec![]), None, None)
                        .map(|Function { body, .. }| {
                            Prop::Getter(GetterProp {
                                span: span!(start),
                                key,
                                body,
                            })
                        }),
                    js_word!("set") => self
                        .parse_fn_args_body(
                            start,
                            |p| p.parse_formal_param().map(|pat| vec![pat]),
                            None,
                            None,
                        ).map(|Function { params, body, .. }| {
                            assert_eq!(params.len(), 1);
                            Prop::Setter(SetterProp {
                                span: span!(start),
                                key,
                                body,
                                param: params.into_iter().next().unwrap(),
                            })
                        }),
                    js_word!("async") => self
                        .parse_fn_args_body(
                            start,
                            Parser::parse_unique_formal_params,
                            Some(ident.span),
                            None,
                        ).map(|function| Prop::Method(MethodProp { key, function })),
                    _ => unreachable!(),
                };
            }
            _ => unexpected!(),
        }
    }
}

#[parser]
impl<'a, I: Input> ParseObject<'a, Pat> for Parser<'a, I> {
    type Prop = ObjectPatProp;

    fn make_object(span: Span, props: Vec<Self::Prop>) -> Pat {
        Pat::Object(ObjectPat { span, props })
    }

    /// Production 'BindingProperty'
    fn parse_object_prop(&mut self) -> PResult<'a, Self::Prop> {
        let start = cur_pos!();

        let key = self.parse_prop_name()?;
        if eat!(':') {
            let value = box self.parse_binding_element()?;

            return Ok(ObjectPatProp::KeyValue(KeyValuePatProp { key, value }));
        }
        let key = match key {
            PropName::Ident(ident) => ident,
            _ => unexpected!(),
        };

        let value = if eat!('=') {
            self.include_in_expr(true)
                .parse_assignment_expr()
                .map(Some)?
        } else {
            if self.ctx().is_reserved_word(&key.sym) {
                syntax_error!(key.span, SyntaxError::ReservedWordInObjShorthandOrPat);
            }

            None
        };

        Ok(ObjectPatProp::Assign(AssignPatProp {
            span: span!(start),
            key,
            value,
        }))
    }
}
