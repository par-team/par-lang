# 조건과 `if`

Par에서는 식 문법과 프로세스 문법 모두 `if`를 지원한다(전자에서는 값을 반환하고, 후자에서는 프로세스 코드를 실행함). Par의 `if`가 특별한 점은 바로 분기를 매치하고 자체적으로 값을 대입시키며, 대입된 변수가 `and`/`or`/`not`을 타고 흐르기도 하는 자체적인 조건 언어에 있다.

우선 위에서 아래로 읽을 수 있는 간단한 `if`식에서 시작해 `if`식에서 지원하는 조건 언어 전체를 배우고, 같은 개념을 프로세스 코드에도 적용해 보자.

## 식 형태의 `if { ... }`

작은 예제부터 시작하자.

```par
module Main

import {
  @core/Bool
  @core/String
}

dec Describe : [Bool] String
def Describe = [flag] if {
  flag => "on",
  else => "off",
}
```

위에서 아래로 읽으면 되는 문법이다.

- `if {`로 조건식을 연다.
- 조건지는 `<condition> => <expression>`의 형태로 작성한다.
- 조건지는 반점으로 구분한다.
- `else => ...`는 폴백 조건지이다.
- 닫는 `}`로 조건식을 닫는다.

요약하면 다음과 같다.

```par
if {
  <condition> => <expression>,
  ...
  else => <expression>,
}
```

`Bool` 타입(`either { .true!, .false! }`)의 값은 직접 조건으로 사용할 수 있으므로, `Bool` 타입을 반환하는 어떤 식이든 `=>`의 좌변에 작성할 수 있다.

다른 언어에서 흔히 쓰이는 `if ... else if ... else` 연쇄와 달리, Par의 '보통' `if`는 임의 개수의 조건지가 있을 수 있는 단일 `if { ... }` 블록이다.

`if { ... }`를 평가할 때에는 다음과 같이 한다.

- 조건지를 위에서부터 차례대로 확인한다.
- 조건지마다 주어진 조건을 평가한다.
- 조건이 성공하는 처음 조건지가 선택되며, `if`식 전체가 그 조건지의 식으로 평가된다.
- 성공하는 조건이 없을 경우에는 `else` 조건지가 선택된다.

## `is`를 사용하여 분기 매칭(과 대입)

Par의 '합 타입'은 [분기](../types/either.md)이다. 추가적인 조건 없이 분기 값에 대해 단순 분기가 필요하다면 `.case`를 사용하면 되지만, 대입을 유지한 채로 매치와 *동시에* 필터링이 필요하다면 `if`를 유용하게 사용할 수 있다.

가장 자주 볼 수 있는 분기 타입으로는 `Try`와 `Option`이 있다.

```par
type Try<e, a> = either {
  .err e,
  .ok a,
}

type Option<a> = either {
  .none!,
  .some a,
}
```

`is` 조건에서는 분기 값을 확인한 뒤 페이로드를 대입하며, 다음과 같은 형태로 작성한다.

```par
<value> is .<variant><payload-pattern>
```

페이로드 패턴은 생략할 수 없다. 단위 페이로드의 경우에도 `!`를 작성해야 하므로, `value is .less!`는 올바르지만 `value is .less`는 그렇지 않다.

내장 비교 연산자는 결과 타입으로 `Ordering`을 반환하는 제네릭 비교 함수를 사용하여 구현되어 있다.

```par
type Ordering = either {
  .less!,
  .equal!,
  .greater!,
}
```

`if`를 사용하여 `Ordering`의 선지를 직접 매치할 수 있다.

```par
module Main

import {
  @core/Data
  @core/Int
  @core/String
}

dec CompareSign : [Int, Int] String
def CompareSign = [x, y]
  let cmp = Data.Compare((x) y) in
  if {
    cmp is .less! => "less",
    cmp is .greater! => "greater",
    else => "equal",
  }
```

역시 위에서 아래로 읽으면 된다.

- `cmp is .less!`가 성공할 경우, `if`식 전체가 `"less"`가 된다.
- 그렇지 않고 `cmp is .greater!`가 성공할 경우, `if`식 전체가 `"greater"`가 된다.
- 그렇지 않으면 `else`로 넘어가서 `"equal"`이 된다.

`is`는 분기 타입만 매치할 수 있다. `is 5`처럼 임의의 값을 매치하는 것은 불가능하며, 이때는 일반적인 부울 등식을 사용하면 된다.

## `and`를 사용하여 매치 좁히기

Par에서는 `and`/`or`/`not`이 단순한 불 대수 연산자가 아니라 제어 흐름 문법으로, 단락 평가를 지원할 뿐만 아니라 `is`로 대입한 변수가 조건지 안이나 다음 조건의 범위에 들어오는지의 여부도 이 연산자의 성공 여부에 달려 있다.

### `and`와 추가 조건

```par
dec CountStatus : [Option<Nat>] String
def CountStatus = [count] if {
  count is .some n and n == 0 => "zero",
  count is .some _ => "non-zero",
  else => "missing",
}
```

차근차근 살펴 보자.

- 첫 번째 조건지를 검사한다.
  - 우선 `count is .some n`을 평가한다. 이 조건이 실패하면 `and` 조건 전체가 실패한다.
  - 성공할 경우, `n`이 대입된 채로 `n == 0`을 평가한다.
  - `and`의 양변이 모두 성공해야만 이 조건지에서 `"zero"`를 반환한다.
- 첫 번째 조건지가 실패할 경우, 두 번째 조건지인 `count is .some _`를 검사한다. 이때는 값이 있는 경우가 모두 매치되고 `"non-zero"`를 반환한다.
- 두 조건지가 모두 실패할 경우, `count`가 `.none!`임을 알 수 있다. `else` 조건지에서 `"missing"`을 반환한다.

### `and`와 다중 대입

```par
dec AddOk : [Try<String, Int>, Try<String, Int>] Int
def AddOk = [left, right] if {
  left is .ok a and right is .ok b => a + b,
  else => 0,
}
```

조건 전체가 성공할 경우, 양쪽 매치가 모두 성공한 것이므로 `a`와 `b` 모두 조건지의 범위 안에 들어온다.

이렇게 읽으면 된다.

- `left`에서 `a` 대입을 시도한다.
- 좌변 대입이 성공한 경우에만 `right`에서 `b` 대입을 시도한다.
- 양변 대입이 모두 성공한 경우에만 `a + b` 조건지가 선택된다.

## `or`를 사용하여 여러 조건 시도

`and`와 같이 `or`도 단락 평가를 지원한다. 즉, 좌변이 실패할 경우에만 우변이 실행된다. 다음과 같이 이해하면 좋다.

> 우선 왼쪽 조건을 확인한다. 실패할 경우에는 오른쪽 조건도 확인해 본다.

`or`의 대표적인 용도로는 '폴백' 매칭이 있다.

```par
dec PickOk : [Try<String, String>, Try<String, String>] String
def PickOk = [primary, fallback] if {
  primary is .ok value or fallback is .ok value => value,
  else => "<missing>",
}
```

2회 시도로 읽으면 된다.

- `primary is .ok value`를 검사한다.
- 좌변이 실패한 경우에만 `fallback is .ok value`를 검사한다.
- 둘 중 하나라도 성공하면 조건지가 선택되어 `value`를 반환한다.

`or` 조건에서 대입된 변수를 사용하려면, 위의 예제와 같이 모든 성공 경로마다 *같은 이름*으로 대입하여야 한다. 양변에서 다른 이름을 사용할 경우, `or` 조건 이후 해당되는 이름의 대입이 보장되지 않는다.

## `{ ... }`를 사용하여 조건 묶기

`not`, `and`, `or`에는 `not` > `and` > `or`의 순서대로 우선순위가 있다. 연산 순서를 확실히 표기하려면 *조건 안에서* `{ ... }`를 사용하면 된다.

```par
dec EmptyOrSpace : [Try<String, String>] Bool
def EmptyOrSpace = [result] if {
  result is .ok s and { s == "" or s == " " }
    => .true!,
  else => .false!,
}
```

이때의 중괄호는 여러 조건을 묶는 역할을 하며, `if { ... }`에서 `if` 다음에 오는, 여러 조건지를 묶는 중괄호와는 다르다.

## 대입이 실패 경로로 흐르는 `not`

`not`은 웬만한 다른 언어와는 다르게 동작한다. 이 연산자는 성공과 실패를 맞바꾸는 동시에 변수 대입에도 영향을 미친다. 즉, 조건 안에서 대입된 변수가 `not`의 *실패* 경로로 흐르게 된다.

```par
dec UseOrError : [Try<String, String>] String
def UseOrError = [result] if {
  not result is .ok value => "error",
  else => value,
}
```

집중해서 읽으면 다음과 같다.

- `result is .ok value`는 `.ok value`일 경우에 성공한다.
- `not`을 사용했으므로 성공과 실패가 뒤집힌다.
- 즉, `else` 조건지가 `result is .ok value`가 성립하는 경우에 해당하고 `value`가 이 조건지에 들어온다.

`or`는 좌변이 실패할 경우에만 우변을 검사하므로, `not`과 `or`를 조합해서 좌변의 대입을 우변으로 '넘겨주도록' 코드를 작성할 수 있다.

```par
dec NonEmptyOrError : [Try<String, String>] String
def NonEmptyOrError = [result] if {
  not result is .ok str or str == "" => "bad input",
  else => str,
}
```

제어 흐름을 따라가 보자.

- 우선 `not result is .ok str`을 평가한다.
- 우변의 `str == ""`은 좌변이 실패할 경우에만 평가된다.
- 좌변이 실패하는 경우는 `result is .ok str`가 성공하는 경우와 일치한다.
- 이 때문에 `str`가 첫 번째 조건지에서 `or`의 우변과 `else` 조건지에 들어오게 된다.

묶은 조건에도 동일하게 사용할 수 있다.

```par
dec AddBothOrZero : [Try<String, Int>, Try<String, Int>] Int
def AddBothOrZero = [left, right] if {
  not { left is .ok x and right is .ok y } => 0,
  else => x + y,
}
```

같은 방법으로 읽으면 된다. `x`와 `y`는 `{ ... }` 안의 조건에서 대입된 것인데, `not`에 의해 두 변수가 `else` 조건지에 들어오게 된다.

### 간단한 비교 (Java `Optional`)

Java에서는 같은 개념이 '체크하고 가져오기'의 두 단계가 되기 일쑤다.

```java
if (result.isEmpty() || result.get().isEmpty()) {
    log("bad input");
    return;
}
var str = result.get();
log(str);
```

Par에서는 매치와 조건을 한 자리에 모아 둘 수 있다.

## 단독 부울식

조건 문법으로 부울 값 역시 직접 계산할 수 있다.

```par
let ok = left or right
```

위와 같은 부울식에서 대입된 변수는 해당하는 식 **안에서만** 머물게 된다.

```par
let ok = result is .ok msg and msg == ""
// `msg`가 조건식 이후로 노출되지 않음
```

조건지 밖에서 대입된 변수를 사용해야 할 때는 `if`식을 사용해야 한다.

## 프로세스 문법에서의 `if`

프로세스 문법에서는 조건지마다 중괄호 안에 *프로세스 코드*를 작성하며, 코드 실행이 `if` 뒤로도 이어질 수 있다.

다중 조건 형태는 다음과 같다.

```par
if {
  <condition> => { <process> }
  ...
  else => { <process> }
}
```

(여기에는 반점이 없으며, `.case { ... }`와 같이 조건지를 공백으로 구분한다.)

아직 프로세스 문법을 배우지 않았다면, [프로세스 문법](../process_syntax.md)을 먼저 읽어 보도록 하자. 짧게 요약하자면, `do { ... } in expr`라고 작성하면 중괄호 안의 프로세스를 실행하고 `in` 다음의 식으로 진행된다.

```par
dec Show : [Try<String, String>] !
def Show = [result] do {
  if {
    result is .ok msg => { Debug.Log(msg) }
    else => { Debug.Log("bad") }
  }
  Debug.Log("after") // 후속 코드
} in !
```

식 형태와 같이 조건지를 위에서부터 차례로 검사하며, 처음으로 성공하는 조건지가 실행된다. `if { ... }`가 종료되고 나면 프로세스는 후속 코드로 계속 이어진다.

### 단일 조건 `if`문

특수한 단일 조건 형태 역시 지원하고 있다.

```par
if <condition> => { <process> }
<more process code>
```

이때 `<more process code>`는 `else` 조건지이자 후속 코드의 역할을 하므로 조기 종료에 이 문법을 활용할 수 있다.

조기 종료는 입력이 잘못되었으면 종료, 아니면 계속 진행하는 '가드'문에 흔히 쓰이는 스타일이다.

```par
dec DefaultIfEmpty : [String] String
def DefaultIfEmpty = [text] chan out {
  if text == "" => {
    out <> "<empty>"
  }
  out <> text
}
```

여기서는 조기에 반환할 수 있도록 `chan`을 사용한다. `out <> "<empty>"` 명령으로 프로세스가 즉시 종료된다.

단일 조건 `if`를 가드로 사용하는 것이 흔한 패턴이다.

```par
dec LogNonEmptyOk : [Try<String, String>] !
def LogNonEmptyOk = [result] chan exit {
  if not result is .ok str or str == "" => { exit! }
  Debug.Log(str)
  exit!
}
```

'입력이 잘못되었으면 종료한다. 그렇지 않으면 `str`가 대입된 상태이고 안전하게 사용할 수 있다'로 이해하면 된다.

## `if { ... }`에서 `else`의 생략

다중 조건 형태의 `if { ... }`는 **모든 경우를 빠짐없이 확인했을 경우에만** `else`를 생략할 수 있다. 이 조건은 타입으로써 확인한다.

예를 들어, 다음 조건식은 `cmp`의 타입이 `Ordering`이므로 문제가 없다.

```par
let cmp = Data.Compare((x) y) in
if {
  cmp is .less! => "less",
  cmp is .equal! => "equal",
  cmp is .greater! => "greater",
}
```

```par
if {
  list is .item(x) xs => { ... }
  list is .end! => { ... }
}
```

프로세스 문법에서는 `if { ... }` 뒤에 오는 코드도 암시적 `else`가 아닌 후속 코드에 불과하다.

> 현재 `if`의 조건 완전성 검사는 완벽하지 않으며, 더 복잡한 조합은 올바르게 검사하지 못할 수 있다.

## 정리

- 식 형태의 `if { ... }`는 값을 반환한다.
- `is`는 `either`를 매치하며, 페이로드 패턴은 항상 작성한다.
- `and`/`or`/`not`은 단락 평가가 이루어지며, `is` 조건의 변수 대입 역시 제어 흐름으로 넘어간다.
- 단독 부울식에서는 변수 대입이 국소적으로 이루어진다.
- 프로세스 문법에서는 다중 조건 `if { ... }`에 후속 코드가 올 수 있다.
- 단일 조건 프로세스 문법에서는 `else`의 경우에 후속 코드가 실행된다.
