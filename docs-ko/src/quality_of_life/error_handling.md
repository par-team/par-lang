# 오류 처리

파일이 없거나, 네트워크 연결이 끊기거나, 사용자가 잘못된 입력을 하는 등, 실 세계와 상호작용하는 프로그램이라면 오류를 깔끔하게 처리해야 한다. 대부분의 오류는 프로그램이 자신의 영역 밖인 시스템과 접촉하는 입출력 경계에서 발생한다.

Par는 명시적은 시도(Try) 값으로써 오류를 표현하고, 이를 활용해 프로세스 간에 오류를 전파하는 가벼운 문법인 `try`/`catch`/`throw`를 제공한다. 마지막으로 오류 전파 시에 범위 안에 자원이 남아 있을 때는 [자동 정리](../types/auto_cleanup.md)로써 올바르게 닫히거나 취소되거나 롤백되도록 한다.

## 오류는 값이다

표준 모듈의 시도 타입은 [분기](../types/either.md)이다.

```par
type Try<e, a> = either {
  .err e,
  .ok a,
}
```

연산이 성공하면 `.ok` 안에 결과값을 담아 반환하고, 실패하면 `.err` 안에 오류 값을 담아 반환한다. 여기에는 예외가 끼어들 여지가 없다.

Par에서 이렇게 하는 것은 동시적인 프로세스는 예외 시 되돌릴 수 있는 호출 스택을 이루지 않기 때문이다. 프로세스는 채널을 통해 통신하므로, 한 프로세스에서 다른 프로세스로 오류를 전파할 때는 프로토콜에 정해진 방법으로 명시적으로 전송해야 한다.

다만 하나의 순차적 프로세스 내부에서 시도 값을 연거푸 매치하는 것은 귀찮은 일이 될 것이다. `try`/`catch`/`throw`문이 바로 이 부분을 담당한다. 이 문법은 국소적인 문법 설탕으로, 예외 형태의 스택 되돌림 없이도 명시적인 시도 값 전파를 편리하게 작성할 수 있도록 한다.

## `try`/`catch`의 첫인상

한 파일의 내용을 다른 파일로 복사하는 아래의 완성된 프로그램을 살펴 보자.

```par
module CopyFile

import {
  @core/Bytes
  @basic/Console
  @basic/Os
}

def Main: ! = chan exit {
  let console = Console.Open

  catch ! => { console.print("Failed to read input."); exit! }
  console.prompt("Src path: ")[try src]
  console.prompt("Dst path: ")[try dst]

  catch e: Os.Error => {
    console.print("An error occurred:")
    console.print(e)
    exit!
  }

  let try reader = src->Os.Path->Os.OpenFile
  let try writer = dst->Os.Path->Os.CreateOrReplaceFile

  reader.begin.read.try.case {
    .end! => {
      writer.close.try
      exit!
    }
    .chunk(bytes) => {
      writer.write(bytes).try
      reader.loop
    }
  }
}
```

첫 번째 `catch`에서는 `console.prompt`에서 반환하는 단위 오류를, 두 번째 `catch`에서는 파일시스템 오류를 처리하고 있다. 한편 `try`문은 시도 값을 매치해서 `.ok` 값은 풀어낸 뒤 계속하고, `.err` 값은 가장 가까운 `catch`로 전달한다.

오류 핸들러에 **없는** 것이 무엇인지 눈치챘는가? 바로 자원을 수동으로 닫는 상용구 코드이다. 목적지 파일을 여는 것이 실패한다면, 이미 열려 있는 출발지 파일이 정리된다. 나중에 복사가 실패한다면, 해당 오류 경로의 범위에 남아 있는 모든 핸들이 알아서 정리된다. `exit!`로 핸들러를 종료한다면 콘솔도 같이 정리된다.

명시적으로 남아 있는 정리 코드는 단 한 줄뿐이다.

```par
writer.close.try
```

쓰기 핸들을 닫을 때는 남아 있는 출력을 비우기 때문에 닫는 연산 자체가 실패할 수 있다. 성공 경로에서도 이 오류는 연산의 결과값에 속하기 때문에 프로그램에서 `try`를 사용해 관찰해야 한다. 자동 정리는 자원을 이미 버리기로 결정했고 정리의 결과를 무시해도 되는 상황에 사용할 수 있는 기능이다.

## 오류 경로에서의 자동 정리

위에서 살펴본 파일 핸들은 선형이지만, 버리는 것이 가능하다. 프로토콜에서 `.close`를 정리 메서드로 표시하고 있기 때문이다.

```par
type Reader<e> = recursive choice {
  .close* => Try<e, !>,
  .read => Try<e, either {
    .end!,
    .chunk(Bytes) self,
  }>,
}

type Writer<e> = iterative choice {
  .close* => Try<e, !>,
  .write(Bytes) => Try<e, self>,
}
```

`.close`의 별표는 이 메서드가 안전하게 객체를 정리하는 표준 메서드라는 의미이다. `throw`나 연결, 탈출 명령을 수행해 이런 값이 버려질 때는 Par에서 이 메서드를 자동으로 선택하고 그 결과값을 마저 정리한다.

`Os.Writer`의 경우, 이 결과값은 `Try<Os.Error, !>`이다. 양쪽 분지 모두 일반적인 데이터이므로 결과값을 무시해도 문제가 없다. 하지만 이 경우에는 자동 정리 중 생성되는 오류 값도 같이 무시한다는 문제가 있다. 닫기 오류를 직접 다뤄야 한다면, 위의 복사 프로그램처럼 명시적으로 `.close`를 호출하고 시도 값을 사용하면 된다.

## 설탕 문법의 의미

그래도 명시적인 코드를 한 번은 살펴보는 것이 도움이 될 것이다. 파일을 연 뒤 그 읽기 핸들을 사용하는 코드를 `try` 없이 프로세스 문법으로 작성하면 다음과 같다.

```par
let result = Os.OpenFile(path)
result.case {
  .err e => {
    console.print(e)
    exit!
  }
  .ok reader => {}
}

// 여기부터 `reader`를 사용할 수 있음
```

`.ok` 분지는 뒤의 코드로 넘어가면서 후속 코드에서 `reader`를 사용할 수 있도록 넘겨준다. `.err` 분지에서는 현재 경로를 종료한다. `try`문은 위의 반복되는 형태를 세 가지 작은 구문으로 감싸는 역할을 한다. 두 가지 형태 중 [프로세스 문법](../process_syntax.md) 형태부터 확인해 보자.

### `catch`문

프로세스 `catch`에서는 전파된 오류를 어떻게 할지 정의한다.

```par
catch <pattern> => {
  <process>
}
```

`<pattern>` 부분에서는 `let`의 좌변과 같은 문법의 패턴을 사용해 오류 값을 대입하여 `catch`의 본문에 전달한다.

```par
catch ! => { ... }
catch e: Os.Error => { ... }
```

`catch`의 본문은 반드시 현재 프로세스 경로를 종료해야 한다. 즉, 후속 코드로 넘어갈 수 없고...

- `continuation!`으로 탈출하거나,
- `left <> right`로 두 채널을 연결하거나,
- 바깥의 `begin`으로 `loop`하거나 (이 경우는 재시도에 적합하다),
- 이전의 `catch`로 `throw`해야 한다.

`try`나 `throw`를 하기 전에는 같은 순차적 프로세스 안에 일치하는 `catch`가 등장해야 한다. `catch`문은 중첩된 식이나 프로세스에는 적용되지 않는다.

### `throw` 명령

`throw`는 오류 값을 `catch` 블록에 직접 전달한다.

```par
catch e => {
  console.print(e)
  exit!
}

throw "Total meltdown"
```

위의 코드는 `e`에 `"Total meltdown"`이 대입된 채로 해당하는 `catch`의 본문이 직접 실행되는 것처럼 동작한다. 이 명령은 기존의 시도 값에서 획득한 오류 값이 아니라 직접 작성한 로직에서 생기는 오류를 다룰 때 적합하다.

### 패턴 형태의 `try`

실패할 수 있는 연산은 대부분 시도 값을 반환한다. 패턴에 `try`를 추가해서 이 값을 매치할 수도 있다.

```par
let try reader = Os.OpenFile(path)
```

이 코드는 다음과 같이 변환된다.

```par
let result = Os.OpenFile(path)
result.case {
  .err e => { throw e }
  .ok reader => {}
}
```

`try` 자체가 패턴 문법이므로 다른 패턴과 합성도 가능하다.

```par
let (try leftReader, try rightReader)! = (
  Os.OpenFile(leftPath),
  Os.OpenFile(rightPath),
)!
```

수신 명령에서 역시 사용할 수 있다. 예를 들어, `Console.prompt`는 콘솔 핸들을 시도 타입으로 감싸서 반환한다.

```par
catch ! => {
  console.print("Failed to read input.")
  exit!
}

console.prompt("What's your name?")[try name]
```

### 명령 형태의 `.try`

프로세스 명령의 주어가 시도 값이 되었을 때는 뒤에 `.try`를 붙여 성공한 분지를 그 자리에서 풀어낼 수 있다.

```par
writer.write("[INFO] Started\n").try
```

위의 코드는 아래의 익숙한 분기를 짧게 줄인 것이다.

```par
writer.write("[INFO] Started\n").case {
  .err e => { throw e }
  .ok => {}
}
```

성공 시에는 `writer`가 `.ok` 안의 값으로 갱신되어 다음 명령에서 사용할 수 있게 된다.

### `try`의 국소성 제한

다음은 잘못된 프로세스 코드이다.

```par
let writer = Os.CreateOrReplaceFile(path).try  // 오류
```

Par는 여러 식을 동시에 평가한다. `let`문은 등호의 우변에 있는 식이 평가되는 것을 기다리지 않고 바로 다음 구문으로 넘어간다. 이 프로세스는 이미 다른 것을 하고 있을 수 있고, 프로세스를 마음대로 중단시키는 것은 위험하므로 중첩된 식에서 `throw`를 하는 것은 불가능하다.

이때는 패턴에서 `try`를 하면 된다.

```par
let try writer = Os.CreateOrReplaceFile(path)
```

이렇게 작성해야 프로세스에서 시도 값을 기다린 뒤 `.ok`나 `.err`를 확인하고, 상황에 맞게 진행하거나 오류를 전파할 수 있다.

## 식 문법에서의 오류 처리

식 문법에도 국소적 형태에 맞춘 `catch`식이 있다.

```par
catch <pattern> => <error result> in <expression using try or throw>
```

예를 들어, 다음과 같이 성공 값은 변환하고 실패는 전파하는 함수를 구현할 수 있다.

```par
catch e => .err e in
let try rawData = source.fetch in
.ok Encode(rawData)
```

프로세스 문법에서와 같이 `try`는 동시적으로 평가되는 중첩 식에서 빠져나오는 것이 불가능하다. 또한 결과값의 어떤 부분이라도 한번 생성된 뒤에는 실행할 수 없다. 즉, 다음 코드는 올바르지 않다.

```par
catch e => .err e in
.ok {result.try + 1}  // 오류: `try`가 중첩 식 안에 있음
```

`try`가 순차적으로 실행되도록 밖으로 빼내야 한다.

```par
catch e => .err e in
let try value = result in
.ok {value + 1}
```

식 형태의 `try`는 오류를 매핑하는 데도 적합하다.

```par
catch e => .err `Failed to process file: #{e}` in
let try content = file.readAll in
.ok ProcessContent(content)
```

## 레이블과 다중 오류 경로

`begin`/`loop`와 같이 `catch` 블록에도 레이블을 추가할 수 있다.

```par
catch@fs e => { /* handle file-system errors */ }
catch@net e => { /* handle network errors */ }

let try@fs writer = path.createFile
let try@net connection = url.connect
```

레이블은 오류 타입이 아니라 거리와 이름을 기준으로 선택된다. `try@fs`와 `throw@fs`는 앞선 `catch@fs` 중 가장 가까운 것을, 레이블이 없는 `try`와 `throw`는 라벨이 없는 `catch` 중 가장 가까운 것과 대응한다.

보통은 `catch` 하나로 충분한 경우가 많지만, 하나의 프로세스에 서로 구분되는 오류 경로가 여럿 있을 때, 혹은 순수 선형 자원을 명시적으로 정리해야 할 때는 레이블을 유용하게 사용할 수 있다.

### 자동 정리가 불가능할 때

모든 선형 객체에 안전한 자동 정리 연산이 주어지는 것은 아니다. 프로토콜에서는 객체 정리에 호출자만 알고 있는 정보가 필요하거나, 결과값을 무시해서는 안 되는 등의 이유로 의도적으로 정리 분지를 정하지 않는 경우가 있다.

```par
type StrictResource = choice {
  .close => !,  // 별표 없음, 이 자원은 순수 선형임
}
```

오류 경로의 범위 안에 `StrictResource`가 남아 있을 경우, 이 자원을 직접 처리하지 않으면 컴파일 오류가 된다. 이때 `catch`문에 레이블을 달아 작은 정리 체인을 만들 수 있다.

```par
catch e => {
  console.print(e)
  exit!
}

let try first = OpenFirst
catch@first e => {
  first.close
  throw e
}

let try@first second = OpenSecond
catch@second e => {
  second.close
  throw@first e
}

Prepare.try@second

second.close
first.close
exit!
```

`OpenSecond`가 실패할 경우, `catch@first`가 `first`를 정리하고 메인 `catch`에 위임한다. `Prepare`가 실패할 경우, `catch@second`가 우선 `second`를 정리하고 `catch@first`에 오류를 전파하며, 여기서 `first`도 정리된다. 성공 경로에서는 두 자원 모두를 명시적으로 정리한다.

## 함수 밖으로 오류 전파

`catch`에서 꼭 오류를 출력하거나 종료할 필요는 없고, 새로운 시도 값을 생성해서 호출자에게 전달해도 된다.

```par
module Main

import {
  @basic/Os
  @core/Bytes
  @core/Try
}

dec ReadAll : [Os.Path] Try<Os.Error, Bytes>
def ReadAll = [path]
  catch e => .err e in
  let try reader = Os.OpenFile(path) in
  Bytes.ReadAll(reader)
```

이 `catch`식은 `Os.OpenFile`의 오류를 함수의 `.err` 결과값으로 변환한다. 성공 시에는 읽기 핸들을 `Bytes.ReadAll`에 넘겨 최종적으로 시도 값을 만든다.

## `default`를 이용한 기본값 삽입

옵션 값이 비어 있을 때, 굳이 오류를 전파하지 않고 폴백 값으로 대체만 하고 싶은 경우도 있다. `default` 문법 설탕으로 이 동작을 구현할 수 있다.

이 문법은 `try`/`catch`와는 별개이다. `try`는 시도 값을 풀어내어 `.err`를 전파하고, `default`는 옵션 값을 풀어내어 `.none!`을 대체하는 문법이다. 시도 값의 오류 타입이 `drop`을 만족하고 오류를 의도적으로 무시할 의향이 있다면, 우선 `Try.ToOption`으로 옵션으로 변환할 수 있다.

후위 형태는 식과 명령에서 모두 사용할 수 있다.

```par
let r1: Option<Int> = .some 7
let r2: Option<Int> = .none!

let x = r1.default(0)  // 7
let y = r2.default(0)  // 0
```

패턴 형태도 있으며, 수신 명령에서도 사용할 수 있다.

```par
let default(0) n = Nat.FromString("oops")
```

이 패턴은 `.some`의 경우 그대로 대입하고, `.none`의 경우 폴백 식을 대입한다.

실용적인 예제를 살펴 보자. 맵을 사용하여 단어가 등장한 횟수를 세되, 없는 키의 경우 `0`에서 시작한다.
```par
dec Counts : [List<String>] List<(String) Nat>
def Counts = [words] do {
  let counts = Map.New(type String, type Nat)
  words.begin.case {
    .end! => {}
    .item(word) => {
      counts.entry(word)[default(0) count]
      counts.put(count + 1)
      words.loop
    }
  }
} in counts.list
```

`counts.entry(word)`는 수신 명령을 통해 `Option<Nat>`을 반환한다. `.some`의 경우는 패턴에서 있는 값을 그대로 대입하고, `.none!`의 경우에는 폴백 값인 `0`을 대신 대입한다.
