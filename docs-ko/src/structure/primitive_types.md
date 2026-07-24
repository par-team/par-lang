# 원시 타입

Par의 다양한 타입을 하나하나 살펴보기 전에, 일반적인 타입 연결사로 이루어지지 않은 *원시 타입*부터 살펴보는 것이 좋겠다.

현재 Par의 원시 타입은 일곱 종류가 있다.

- **`Nat`** -- 0부터 시작하는 임의 크기의 자연수. `Int`의 서브타입이다.
- **`Int`** -- 양수와 음수를 포함하는 임의 크기의 정수
- **`Float`** -- IEEE-754 배정밀도 부동소수점 실수
- **`String`** -- UTF-8로 인코딩된 유니코드 문자열. `Bytes`의 서브타입이다.
- **`Char`** -- 단일 유니코드 문자. `String`의 서브타입이다.
- **`Byte`** -- 단일 8비트 값. `Bytes`의 서브타입이다.
- **`Bytes`** -- 연속된 바이트열

Par의 구조적 타입 시스템은 원시 타입까지는 미치지 못한다. 사용자 정의 타입은 단순 동의어로, 타입의 모양이 바로 의미가 된다. 한편 원시 타입은 효율적인 런타임 표현과 특수한 연산을 요구하므로 불투명하다.

## 리터럴

원시 리터럴은 항상 사용할 수 있으며, 별도의 가져오기가 필요 없다.

```par
def Natural = 42       // Nat
def Integer = -7       // Int
def Floating = 3.14    // Float
def Text = "Hello"     // String
def Character = "H"    // Char
def OneByte = <<65>>   // Byte
def ManyBytes = <<65 66 67>>  // Bytes
```

정수와 자연수는 가독성 향상을 위해 밑줄을 삽입할 수 있다.

```par
def Million = 1_000_000
```

부동소수점 리터럴은 소수 부분이 있으며, 지수 표기도 사용할 수 있다.

```par
def Piish = 3.14
def Half = 0.5
def Avogadroish = 6.02e23
```

문자열은 큰따옴표 안에 적으며, 일반적인 이스케이프 시퀀스를 사용한다.

```par
def Greeting = "Hello\nWorld"
```

`Char` 리터럴은 정확히 한 글자로 이루어진 문자열 리터럴을 사용하면 된다.

```par
def Letter = "a"
def Newline = "\n"
```

바이트와 바이트열 리터럴은 겹부등호 안에 적는다. 1바이트의 리터럴은 `Byte`로, 여러 바이트나 빈 리터럴은 `Bytes`로 추론된다.

```par
def A = <<65>>
def ABC = <<65 66 67>>
def Empty = <<>>
```

바이트 값은 256으로 나눈 나머지로 저장되므로 범위를 벗어난 바이트 리터럴은 오버플로우가 된다.

## 연산자

수 타입은 보통 헬퍼 함수를 가져오는 대신 연산자를 사용해 조작한다.

```par
def Arithmetic = 1 + 2 * 3       // = 7
def Grouped = {1 + 2} * 3        // = 9
def Ratio = 22.0 / 7.0
def Difference = 10 - 3
def Negative = neg 5
```

`+`, `*`, `/` 연산자는 `Nat`, `Int`, `Float`를 지원한다. `-`, `neg` 연산자는 부호 있는 수인 `Int`와 `Float`를 지원한다.

원시 값에 대한 비교도 가능하다.

```par
def Smaller = 3 < 10          // = .true!
def SameText = "hi" == "hi"   // = .true!
def Different = "a" != "b"    // = .true!
```

비교의 결과는 아래에서 설명할 `Bool`이 된다. 비교 연산자는 연쇄 역시 가능하다.

```par
def InRange = 0 <= 5 < 10
```

비교 연산자를 연쇄하면 눈에 보이는 대로 동작하며, 위의 식은 `0 <= 5 and 5 < 10`의 의미를 가지는 대신 가운데의 식은 한 번만 평가된다.

[프로세스 문법](../process_syntax.md)에서는 추가로 산술 연산자로 복합 대입이 가능하며, `+=`, `-=`, `*=`, `/=`이 이에 해당한다.

```par
def Five = do {
  let n = 2
  n += 3  // `let n = n + 3`과 같음
} in n
```

## 부울

`Bool`은 원시 타입은 아니고, `@core/Bool`의 일반적인 [분기](../types/either.md) 타입이다.

```par
type Bool = either {
  .false!,
  .true!,
}
```

부울 값은 `.true!`나 `.false!`로 작성한다.

```par
def Yes = .true!
def No = .false!
```

부울 식에는 `and`, `or`, `not`을 사용할 수 있다.

```par
def Yes : Bool = .true!
def No : Bool = .false!

def Both = Yes and No
def Either = Yes or No
def Neither = not Yes
```

위의 키워드를 조건문에서 사용하면 단락 평가가 가능하며, 패턴 매치에서 대입된 변수를 그대로 사용할 수 있다는 추가적인 장점이 있다. 해당 기능은 [조건과 `if`](../quality_of_life/if.md)에서 다룬다.

## 템플릿 문자열

백틱을 사용하는 문자열은 템플릿 문자열이다. 템플릿 문자열도 `String` 값이지만, 추가로 문자열 보간을 지원한다.

`${...}`으로 원래부터 `String` 타입을 가지는 식을 삽입할 수 있다.

```par
def Name = "Ada"
def Greeting = `Hello, ${Name}!`
```

`#{...}`으로 데이터로서 출력할 수 있는 모든 값을 삽입할 수 있다.

```par
def Count = 3
def Message = `You have #{Count} messages.`
```

`#{...}` 문법은 배후에서 `@core/Data.ToString`을 사용하며, 원시 타입을 포함해 순서쌍, 분기, 데이터의 리스트 등 일반적인 자료구조를 모두 지원한다.

템플릿 문자열은 여러 줄을 차지할 수 있으며, 일반적인 이스케이프 시퀀스를 지원한다. 문자열 보간 문법과 겹치는 문자열을 작성하려면 이스케이프 처리를 하면 된다.

```par
def LiteralPieces = `Use \` for backticks, \${ for string interpolation, and \#{ for data.`
```

## 원시 타입의 사용

리터럴은 별도의 가져오기가 필요 없지만, 타입 이름을 직접 사용할 때는 가져오기가 필수이다.

```par
module Main

import {
  @core/Int
  @core/String
}

def Age: Int = 42
def Name: String = "Ada"
```

가져온 모듈을 통해 헬퍼 함수 역시 사용할 수 있다.

```par
module Main

import {
  @core/Int
  @core/Nat
}

def Magnitude = Int.Abs(-1000)
def Remainder = Int.Mod(-13, 5)
def Numbers = Nat.Range(0, 5)  // *(0, 1, 2, 3, 4)
```

모든 패키지는 자동으로 `@core`에 의존하지만, `@core`의 모듈을 자동으로 가져오는 것은 아니다.

## 유용한 원시 모듈

원시 모듈에서는 기초적인 산술이나 비교 이상의 다양한 연산을 지원한다. 아래의 예제는 맛보기로, `par doc`을 통해 내장 API 전체를 확인해볼 수 있다.

### `@core/Nat`

`Nat`는 유한 번 반복이나 자연수 범위 등의 헬퍼 함수를 지원한다.

```par
module Main

import @core/Nat

def ThreeSteps = Nat.Repeat(3)
def ZeroToFour = Nat.Range(0, 5)
```

`Nat.Repeat(n)`은 정확히 `n`개의 단계를 거치는 재귀 값을 생성하며, 미리 계산된 횟수만큼 반복하고자 할 때 자주 사용한다.

### `@core/Int`

`Int`의 헬퍼 함수 중에는 반환 타입이 단순히 '다른 수 타입'이 아닌 것도 있다.

```par
module Main

import {
  @core/Nat
  @core/Int
}

def Absolute: Nat = Int.Abs(-12)
def Modulo: Nat = Int.Mod(-13, 5)
def FromTo = Int.Range(-2, 3)
```

### `@core/Float`

`Float`는 상수, 변환 함수, 조건 함수, 수학적 함수를 지원한다.

```par
module Main

import @core/Float

def Tau = 2.0 * Float.Pi
def Root = Float.Sqrt(9.0)
def Rounded = Float.Round(3.6)
def CloseEnough = Float.Equals(1.0, 1.05, 0.1)
```

데이터를 직접 비교하는 `==`와 달리 `Float.Equals`는 추가로 오차 범위를 지정할 수 있다.

### `@core/String`

문자열을 점진적으로 구성할 수 있다.

```par
module Main

import @core/String

def Hello = String.Builder
  .add("Hello")
  .add(", ")
  .add("world!")
  .build
```

문자열을 `String.Parse`로 파싱하는 것 역시 가능하다. 문자열 파서는 더 복잡한 도구로, 문자열을 읽거나 패턴을 찾고자 할 때 쓰인다.

```par
module Main

import @core/String

def ParserFromText = String.Parse("abc")
```

### `@core/Char`와 `@core/Byte`

`Char.Is`와 `Byte.Is`로 문자나 바이트가 특정한 부류에 속하는지 확인할 수 있다.

```par
module Main

import {
  @core/Byte
  @core/Char
}

def Space = Char.Is(" ", .whitespace!)
def HighByte = Byte.Is(<<192>>, .range(<<128>>, <<255>>)!)
```

### `@core/Bytes`

`Bytes`의 리더, 파서, 빌더를 바이트 지향 프로토콜에 사용할 수 있다.

```par
module Main

import @core/Bytes

def EmptyReader = Bytes.Reader(<<>>)
def Size = Bytes.Length(<<65 66 67>>)
```

원시 타입은 이 정도로 충분히 다루었다. 문서의 나머지 부분에서는 대부분의 Par 프로그램을 이루는 구조적 타입에 대해 알아보자.
